# DLT-TUI 品質レビュー・修正計画（2026-09-09）

## 目標と完了条件

現行のbinary-only構成とRust 1.88対応を維持し、確認できた不具合・監査失敗・資料の不一致を修正する。
通常テスト、format、Clippy、MSRV、除外なしRustSec監査、package検証で確認する。
リリース、マージ、GitHub設定変更はこのローカル改善とは別の公開判断とする。

## 分担と優先順

| 優先 | 対象 | 確認した問題・対応 | 担当 |
| --- | --- | --- | --- |
| P1 | セキュリティ | lru 0.16.3のRUSTSEC-2026-0253で週次監査が失敗。ratatui-core 0.1.2 / lru 0.18.4へ更新し、前回の未公開ignoreを撤去 | 主担当 |
| P1 | 実行時エラー | mainがrun_appのI/Oエラーを標準出力へ表示して成功終了。エラーを呼び出し元へ伝播する | 主担当 |
| P1 | export | exists確認後のFile::createによる競合上書きをcreate_newで防止。既存ファイル保持テスト追加 | Sol・完了 |
| P2 | TCP | ファイルと同じ共有カウンタで復旧時のskip byte数をUIへ反映。localhost回帰テスト追加 | Sol・完了 |
| P2 | 仕様・利用手順 | 実装済みTCP/テキスト出力と将来計画を分離。読込上限・フィルタ保存・ハイライトの記述を実装に合わせる | Luna |
| P2 | CI | 監査に15分timeout。既存3 OS、package、MSRV、Clippy、formatは維持 | 主担当 |

## 現状確認

- 変更前テストは153 passed / 1 ignored。sandbox内のTCP bind拒否は権限付き再実行で解消。
- 2026-09-07のSecurity auditは失敗。ローカル除外なし監査でlru警告を再現。
- 依存更新後の除外なし監査は成功。
- master保護をAPIで確認：6 required checks、最新master追従、管理者にも適用、未解決会話禁止、force push/削除禁止。
- RustSecは現在masterのrequired checkに含まれず、path filter付き。必須化するなら毎PRに結果を返す設計と保護設定変更を同時に行う必要がある。今回は現状を記録し、設定変更は保留。

## 検証・残課題

統合テストは155 passed / 1 ignored（修正前153）。formatとdiff check成功。
最終差分に対し `cargo clippy --all-targets --locked -- -D warnings`、
`cargo +1.88.0 check --all-targets --locked`、`cargo package --locked --allow-dirty` が成功。
`cargo audit --deny warnings` も除外なしで成功。workflow YAMLはRubyでparse確認済み。
actionlintと更新後の3 OS GitHub CIは未実行。
Lunaの独立レビューでエラー伝播、依存更新、MSRV、資料整合性を確認。
package一覧にprogress.mdとレビュー資料が入らないことを確認。
再現根拠のない最適化、大規模設計変更、新機能追加は含めない。

## 次の修正計画

1. **非Verbose DLTのMessage ID（P1・仕様対応の不足）**: 現行parserはpayload全体を簡易テキスト化し、Message IDを独立に扱わない。単純に4バイトを捨てる変更ではControl等のmessage typeとの区別を壊しかねない。MSTP、byte order、raw保存、表示・検索・exportの意味を揃えた上で、実規格に沿うLE/BE fixtureで修正する。辞書なしの値復号は今回の機能範囲外。参照: [COVESA non-verbose API](https://github.com/COVESA/dlt-daemon/blob/master/doc/dlt_for_developers.md#verbose-vs-non-verbose-api)。
2. **空・全破損入力の案内（P2）**: ロード完了時0件の理由を明示する。正常な空ファイルと破損、I/O失敗を区別するテストを先に追加。
3. **RustSecをPR必須条件にする（P2）**: path filterを外して毎PRで結果を返すようにし、required checksへRustSecを追加。外部保護設定と同時に反映する。
4. **実機・負荷検証（P2）**: Linux/Windows/macOSのCI結果に加え、代表的な大きさの匿名化ログでUI操作・RAM・ロード時間を測定する。現時点で実機GUI/PTYの全操作や性能改善を検証したとはしない。

## 公開状態

修正はローカルのみ。既存GitHub Actionsの失敗履歴・通知は、この差分をmasterへ反映するまで解消しない。
crates.io / Homebrewの公開版も未変更。次リリースで本修正を配布する。
