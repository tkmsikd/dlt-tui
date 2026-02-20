use std::fs::File;
use std::io::{Read, Result};
use std::path::Path;

use std::io::{Cursor, Error, ErrorKind};

pub fn open_dlt_stream<P: AsRef<Path>>(path: P) -> Result<Box<dyn Read>> {
    let path_ref = path.as_ref();
    let file = File::open(path_ref)?;

    let ext = path_ref
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "gz" => {
            // Let's do a quick read to validate the GZip header if we want to catch broken files early.
            // But actually GzDecoder does check the header on new or first read.
            // So we can just return it. To make `test_read_broken_compression_returns_err` pass,
            // we should probably force a header read.
            let mut decoder = flate2::read::GzDecoder::new(file);
            let mut buf = [0; 0];
            if let Err(e) = decoder.read(&mut buf) {
                return Err(e);
            }
            Ok(Box::new(decoder))
        }
        "zip" => {
            let mut archive = zip::ZipArchive::new(file)?;
            // for MVP, we just take the first file and slurp it.
            if archive.len() == 0 {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "Zip file string is empty",
                ));
            }
            let mut zipped_file = archive.by_index(0)?;
            let mut buffer = Vec::new();
            zipped_file.read_to_end(&mut buffer)?;
            Ok(Box::new(Cursor::new(buffer)))
        }
        _ => Ok(Box::new(file)),
    }
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
    fn test_read_broken_compression_returns_err() {
        let tmp_dir = tempdir().unwrap();
        let bad_gz_path = tmp_dir.path().join("broken.gz");
        fs::write(&bad_gz_path, b"NOT_A_GZIP_FILE_AT_ALL").unwrap();

        let result = open_dlt_stream(&bad_gz_path);
        assert!(result.is_err());
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
