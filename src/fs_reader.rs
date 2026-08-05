use std::{
    fs::File,
    io::{Cursor, Error, ErrorKind, Read, Result},
    path::Path,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
};

const MAX_LOAD_SIZE: u64 = 500 * 1024 * 1024; // 500MB max per file
const ZIP_CHUNK_SIZE: usize = 64 * 1024;

pub fn open_dlt_stream<P: AsRef<Path>>(path: P) -> Result<Box<dyn Read>> {
    open_dlt_stream_with_limit(path.as_ref(), MAX_LOAD_SIZE)
}

fn open_dlt_stream_with_limit(path_ref: &Path, limit: u64) -> Result<Box<dyn Read>> {
    let file = File::open(path_ref)?;

    let ext = path_ref
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "gz" => {
            let mut decoder = flate2::read::MultiGzDecoder::new(file);
            let mut buf = [0; 0];
            #[allow(clippy::unused_io_amount)]
            decoder.read(&mut buf)?;
            Ok(Box::new(SizeLimitedReader::new(decoder, limit)))
        }
        "zip" => {
            let mut archive = zip::ZipArchive::new(file)?;
            let mut selected_index = None;
            for index in 0..archive.len() {
                if archive.by_index(index)?.is_file() {
                    selected_index = Some(index);
                    break;
                }
            }

            let selected_index = selected_index.ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidData,
                    "Zip archive contains no file entries",
                )
            })?;
            if archive.by_index(selected_index)?.size() > limit {
                return Err(size_limit_error(limit));
            }

            let (sender, receiver) = sync_channel(2);
            std::thread::spawn(move || {
                let event = match stream_zip_entry(archive, selected_index, limit, &sender) {
                    Ok(()) => ZipEvent::End,
                    Err(error) => ZipEvent::Error(error),
                };
                let _ = sender.send(event);
            });
            Ok(Box::new(ChannelReader::new(receiver)))
        }
        _ => {
            if file.metadata()?.len() > limit {
                return Err(size_limit_error(limit));
            }
            Ok(Box::new(SizeLimitedReader::new(file, limit)))
        }
    }
}

enum ZipEvent {
    Data(Vec<u8>),
    End,
    Error(Error),
}

fn stream_zip_entry(
    mut archive: zip::ZipArchive<File>,
    selected_index: usize,
    limit: u64,
    sender: &SyncSender<ZipEvent>,
) -> Result<()> {
    let zipped_file = archive.by_index(selected_index)?;
    let mut reader = SizeLimitedReader::new(zipped_file, limit);
    loop {
        let mut chunk = vec![0; ZIP_CHUNK_SIZE];
        let read = match reader.read(&mut chunk) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            return Ok(());
        }
        chunk.truncate(read);
        if sender.send(ZipEvent::Data(chunk)).is_err() {
            return Ok(());
        }
    }
}

struct ChannelReader {
    receiver: Receiver<ZipEvent>,
    current: Cursor<Vec<u8>>,
    finished: bool,
}

impl ChannelReader {
    fn new(receiver: Receiver<ZipEvent>) -> Self {
        Self {
            receiver,
            current: Cursor::new(Vec::new()),
            finished: false,
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() || self.finished {
            return Ok(0);
        }

        loop {
            if self.current.position() < self.current.get_ref().len() as u64 {
                return self.current.read(buf);
            }

            match self.receiver.recv() {
                Ok(ZipEvent::Data(chunk)) => {
                    self.current = Cursor::new(chunk);
                }
                Ok(ZipEvent::End) => {
                    self.finished = true;
                    return Ok(0);
                }
                Ok(ZipEvent::Error(error)) => {
                    self.finished = true;
                    return Err(error);
                }
                Err(_) => {
                    self.finished = true;
                    return Err(Error::new(
                        ErrorKind::UnexpectedEof,
                        "ZIP decompression ended without a completion event",
                    ));
                }
            }
        }
    }
}

struct SizeLimitedReader<R> {
    inner: R,
    remaining: u64,
    limit: u64,
}

impl<R> SizeLimitedReader<R> {
    fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            remaining: limit,
            limit,
        }
    }
}

impl<R: Read> Read for SizeLimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.remaining > 0 {
            let allowed = usize::try_from(self.remaining)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            let read = self.inner.read(&mut buf[..allowed])?;
            self.remaining -= read as u64;
            return Ok(read);
        }

        let mut probe = [0u8; 1];
        match self.inner.read(&mut probe) {
            Ok(0) => Ok(0),
            Ok(_) => Err(size_limit_error(self.limit)),
            Err(error) => Err(error),
        }
    }
}

fn size_limit_error(limit: u64) -> Error {
    Error::new(
        ErrorKind::InvalidData,
        format!("DLT input exceeds the configured size limit of {limit} bytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::ZipWriter;

    fn read_all_with_limit(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
        let mut stream = open_dlt_stream_with_limit(path, limit)?;
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    fn gzip_member(data: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn test_read_uncompressed_dlt() {
        let tmp_dir = tempdir().unwrap();
        let dlt_path = tmp_dir.path().join("normal.dlt");
        let dummy_data = b"DLT_DUMMY_DATA";
        fs::write(&dlt_path, dummy_data).unwrap();

        let mut stream = open_dlt_stream(&dlt_path).unwrap();
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).unwrap();
        assert_eq!(buffer, dummy_data);
    }

    #[test]
    fn test_read_gzip_compressed_dlt() {
        let tmp_dir = tempdir().unwrap();
        let gz_path = tmp_dir.path().join("compressed.gz");
        let dummy_data = b"DLT_DUMMY_DATA_GZIPPED";

        let file = fs::File::create(&gz_path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(dummy_data).unwrap();
        encoder.finish().unwrap();

        let mut stream = open_dlt_stream(&gz_path).unwrap();
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).unwrap();
        assert_eq!(buffer, dummy_data);
    }

    #[test]
    fn test_read_concatenated_gzip_members_with_combined_limit() {
        let tmp_dir = tempdir().unwrap();
        let gzip_path = tmp_dir.path().join("concatenated.gz");

        let mut compressed = gzip_member(b"DLT_FIRST");
        compressed.extend(gzip_member(b"DLT_SECOND"));
        fs::write(&gzip_path, compressed).unwrap();

        let expected = b"DLT_FIRSTDLT_SECOND";
        assert_eq!(
            read_all_with_limit(&gzip_path, expected.len() as u64).unwrap(),
            expected
        );

        let error = read_all_with_limit(&gzip_path, expected.len() as u64 - 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains(&format!("size limit of {} bytes", expected.len() - 1))
        );
    }

    #[test]
    fn test_concatenated_gzip_reports_truncated_later_member() {
        let tmp_dir = tempdir().unwrap();
        let gzip_path = tmp_dir.path().join("truncated-second-member.gz");

        let mut compressed = gzip_member(b"DLT_FIRST");
        let mut second = gzip_member(&b"DLT_SECOND".repeat(1024));
        second.truncate(second.len() / 2);
        compressed.extend(second);
        fs::write(&gzip_path, compressed).unwrap();

        assert!(read_all_with_limit(&gzip_path, 1024 * 1024).is_err());
    }

    #[test]
    fn test_read_zip_compressed_dlt() {
        let tmp_dir = tempdir().unwrap();
        let zip_path = tmp_dir.path().join("archive.zip");
        let dummy_data = b"DLT_DUMMY_DATA_ZIPPED";

        let file = fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        // Using SimpleFileOptions or default. zip 0.6 uses FileOptions::default()
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("logfile.dlt", options).unwrap();
        zip.write_all(dummy_data).unwrap();
        zip.finish().unwrap();

        let mut stream = open_dlt_stream(&zip_path).unwrap();
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).unwrap();
        assert_eq!(buffer, dummy_data);
    }

    #[test]
    fn test_zip_streams_large_entry_across_small_reads() {
        let tmp_dir = tempdir().unwrap();
        let zip_path = tmp_dir.path().join("streamed.zip");
        let expected = b"DLT_STREAM_CHUNK".repeat((ZIP_CHUNK_SIZE * 3) / 16 + 1);

        let file = fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("trace.dlt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&expected).unwrap();
        zip.finish().unwrap();

        let mut stream = open_dlt_stream(&zip_path).unwrap();
        let mut actual = Vec::new();
        let mut small_buffer = [0; 17];
        loop {
            let read = stream.read(&mut small_buffer).unwrap();
            if read == 0 {
                break;
            }
            actual.extend_from_slice(&small_buffer[..read]);
        }

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_zip_skips_directory_entries_before_first_file() {
        let tmp_dir = tempdir().unwrap();
        let zip_path = tmp_dir.path().join("directory-first.zip");
        let dummy_data = b"DLT_AFTER_DIRECTORY";

        let file = fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.add_directory("logs/", options).unwrap();
        zip.start_file("logs/trace.dlt", options).unwrap();
        zip.write_all(dummy_data).unwrap();
        zip.finish().unwrap();

        assert_eq!(read_all_with_limit(&zip_path, 1024).unwrap(), dummy_data);
    }

    #[test]
    fn test_zip_with_only_directories_returns_error() {
        let tmp_dir = tempdir().unwrap();
        let zip_path = tmp_dir.path().join("directories-only.zip");

        let file = fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.add_directory("logs/", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();

        let error = read_all_with_limit(&zip_path, 1024).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("no file entries"));
    }

    #[test]
    fn test_size_limit_accepts_exact_size_and_rejects_overflow() {
        let tmp_dir = tempdir().unwrap();
        let exact_path = tmp_dir.path().join("exact.dlt");
        let oversized_path = tmp_dir.path().join("oversized.dlt");
        fs::write(&exact_path, b"1234").unwrap();
        fs::write(&oversized_path, b"12345").unwrap();

        assert_eq!(read_all_with_limit(&exact_path, 4).unwrap(), b"1234");
        let error = read_all_with_limit(&oversized_path, 4).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("size limit of 4 bytes"));
    }

    #[test]
    fn test_size_limit_applies_to_decompressed_gzip_and_zip_data() {
        let tmp_dir = tempdir().unwrap();
        let gzip_path = tmp_dir.path().join("oversized.gz");
        let zip_path = tmp_dir.path().join("oversized.zip");

        let gzip_file = fs::File::create(&gzip_path).unwrap();
        let mut encoder = GzEncoder::new(gzip_file, Compression::default());
        encoder.write_all(b"12345").unwrap();
        encoder.finish().unwrap();

        let zip_file = fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(zip_file);
        zip.start_file("trace.dlt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"12345").unwrap();
        zip.finish().unwrap();

        for path in [&gzip_path, &zip_path] {
            let error = read_all_with_limit(path, 4).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData);
            assert!(error.to_string().contains("size limit of 4 bytes"));
        }
    }

    #[test]
    fn test_read_broken_compression_returns_err() {
        let tmp_dir = tempdir().unwrap();
        let bad_gz_path = tmp_dir.path().join("broken.gz");
        fs::write(&bad_gz_path, b"NOT_A_GZIP_FILE_AT_ALL").unwrap();

        let result = open_dlt_stream(&bad_gz_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_truncated_gzip_returns_error_while_reading() {
        let tmp_dir = tempdir().unwrap();
        let gzip_path = tmp_dir.path().join("truncated.gz");

        let file = fs::File::create(&gzip_path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(&b"DLT_PAYLOAD".repeat(1024)).unwrap();
        encoder.finish().unwrap();

        let mut compressed = fs::read(&gzip_path).unwrap();
        compressed.truncate(compressed.len() / 2);
        fs::write(&gzip_path, compressed).unwrap();

        let mut stream = open_dlt_stream(&gzip_path).unwrap();
        let mut buffer = Vec::new();
        assert!(stream.read_to_end(&mut buffer).is_err());
    }

    #[test]
    fn test_read_empty_file() {
        let tmp_dir = tempdir().unwrap();
        let empty_path = tmp_dir.path().join("empty.dlt");
        fs::write(&empty_path, b"").unwrap();

        let mut stream = open_dlt_stream(&empty_path).unwrap();
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).unwrap();
        assert!(buffer.is_empty());
    }
}
