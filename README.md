# Simple VCS

Rust / Tauri / React / TypeScriptで作る、ローカルとSSH先の作業コピーを操作するデスクトップGUIです。

SVNとGitの作業コピーをローカルまたはSSH先で操作できます。SVNは変更一覧・差分・履歴と基本書込み、Gitは状態・差分・履歴・ブランチ、stage/unstage・commit・fetch・ff-only pull・pushに対応します。

## 起動

Node.js 24 LTS、Rustのstable toolchain、OS別の[Tauri開発要件](https://v2.tauri.app/start/prerequisites/)が必要です。今回の確認版は [docs/VALIDATION.md](docs/VALIDATION.md) に記載しています。

WindowsではMSVCツールチェーン、Visual Studio C++ Build Tools、WebView2を用意してください。macOSはXcode Command Line Toolsが必要です。

```sh
npm ci
npm run tauri dev
```

ブラウザだけの `npm run dev` ではVCS操作できません。Tauriが必要であることを画面に表示します。

## 作業コピーを登録

1. 既存のSVN／Git作業コピーを用意します。checkout/cloneは既存CLI等で行ってください。
2. 「作業コピーを登録」でSVNまたはGit、ローカルまたはSSH先を選択し、名前とその実行先の絶対パスを入力します。
3. ローカルはそのPCのVCS CLI、SSH先は接続先LinuxのCLIを使います。Git用CLIは「CLI設定」で変更できます。ローカルだけの利用にSSH設定は不要です。
4. 変更ファイルを選ぶと差分、履歴からコミットを選ぶと過去の差分を表示します。

CLIを検出できない場合は「CLI設定」で `svn.exe`／`ssh.exe` の絶対パスを指定してください。macOSでローカルSVNが既定の `svn` の場合は、既存PATHに加えてHomebrewの標準配置（`/opt/homebrew/bin`、`/usr/local/bin`）も検索します。独自の場所へ導入した場合や旧版アプリでは、例えば `/opt/homebrew/bin/svn` を直接指定してください。明示した実行ファイルの設定は自動変更しません。引数は実行ファイルの欄へ入力しません。SSH先のSVNパスは接続先の非対話SSHのPATHから探します。

Git CLIも同じ「CLI設定」で指定できます。SSH先では接続先LinuxのPATHから `git` を起動します。

登録内容はOSのアプリ設定ディレクトリの `dev.simple-vcs.desktop/settings.json` 相当の場所へ保存します。秘密鍵・パスワードは保存しません。破損・未対応版の設定は自動初期化しません。登録解除は一覧からの削除のみで、実ファイルを削除しません。

## 書込み操作

1. 変更一覧のチェックボックスで対象を選びます。変更のないファイルを削除予定にする場合は「通常ファイルも表示」を使います。
2. 操作を選び、コミットならメッセージを入力して「対象を確認」を押します。
3. 実行先・対象一覧・影響を確認し、確認チェックを入れて実行します。確認後に状態・差分が変わった場合は実行を止めます。
4. コミット成功時はリビジョンを表示し、状態を再取得します。SVNコミットはリポジトリへの送信です。

ディレクトリの追加・コミット・変更破棄は自身だけを対象とし、子を自動選択しません。追加予定の親や、削除予定ディレクトリの子など、必要な対象が足りないときは選択し直してください。削除操作は配下の対象を確認画面へ展開し、変更・未追跡・無視ファイルがある場合は拒否します。強制削除は行いません。

成否を確認できない場合は自動再実行せず、未完了記録を残します。次回起動後も確認欄を表示します。「状態・最新履歴を確認」に加え、実行先で処理が停止したことと成否を外部で確認してから、書込み制限を解除してください。解除操作は処理の再実行ではありません。

同じ設定を使うアプリの二重起動と、書込み中の通常のウィンドウ終了を防止します。強制終了した場合にも未完了記録から確認できます。外部エディタ・端末・別ユーザーとの同時変更を完全には防げないため、書込み中の同時操作は避けてください。

## Git操作

- 変更一覧はステージ済み・未ステージを分けて表示します。ファイル単位でStage／Unstageし、コミットではステージ済みの変更すべてが対象になることを確認します。
- 未ステージの追跡済み変更は選択して破棄できます。ステージ済み変更と未追跡ファイルはこの操作で削除しません。
- ブランチの作成・切替、Fetch、fast-forwardのみのPull、Pushを別操作にしています。Push／Pull前に作業コピーと送信先を確認します。追跡先がない場合はremote名とbranch名を指定します。
- commitは作業コピーのGitリポジトリへの記録、Pushはremoteへの送信です。SSH先のGitはLinux側CLI・資格情報を使います。
- merge/rebase等の進行中操作、競合、submoduleは通常操作を停止し、外部での処理を案内します。force push、reset、clean、自動stashは行いません。

## SSH先の利用

Windows版OpenSSHを基準とし、ユーザーの `.ssh/config` にHost別名を設定します。

```sshconfig
Host team-server
    HostName linux.example.com
    User developer
    Port 22
    IdentityFile ~/.ssh/id_ed25519
```

アプリには `team-server` と、例えば `/srv/projects/example` を登録します。`user@host` やポートをHost欄へ直接入力せず、configへ指定してください。

- 同じSSH実行ファイル・ユーザーで、事前に端末から接続できることを確認します。パスフレーズ付き鍵はagentに登録してください。
- 初回ホスト鍵は信頼できる経路で指紋を確認して端末で登録します。アプリは未知・変更された鍵を自動承認しません。
- SSH先には選択したSVN／Git CLI、POSIX互換シェル、GNU `realpath -m`、`C.UTF-8` ロケールが必要です。対話入力、TTY、agent forwardingを使いません。
- Windows→Linuxの認証と、LinuxのSVN→リポジトリサーバーの認証は別です。後者はLinux側で設定してください。
- SSH先にファイルは置いたままで、Windowsへの同期は行いません。

## 検証とビルド

Rust統合テストには `svn` と `svnadmin` がPATH上に必要です。テストは一時ディレクトリに使い捨てリポジトリを作り、ユーザーの作業コピーを変更しません。

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm test
npm run build
npm run tauri build
```

Tauri buildとcargo testは、同じtargetディレクトリで同時実行せず、順番に実行してください。

## GitHub Release

`main` へpushすると、[Release workflow](.github/workflows/release.yml) がテスト、採番・タグ作成、各OSのビルド、GitHub Release公開を順番に行います。すべてのビルドと配布物の確認が成功するまでReleaseは下書きです。SHA-256チェックサムも添付します。

| OS | CPU | 配布形式 |
| --- | --- | --- |
| Windows | x64 | ポータブル `.zip`、NSIS `.exe`、`.msi` |
| macOS | Apple Silicon / Intel | CPU別ポータブル `.zip`、`.dmg` |
| Linux | x64 | ポータブル `.zip`、`.deb`、`.AppImage` |

ポータブル版は展開後、`PORTABLE` ファイルとアプリを同じフォルダに置いて起動してください。作業コピー登録、CLI設定、未完了操作の記録はそのフォルダに保存され、フォルダごと移動できます。WindowsではWebView2、利用するSVN／Git、OpenSSHを別途用意してください。macOS/Linuxでも利用するSVN／GitとSSH先操作用のOpenSSHが必要です。設定や記録の保護のため、書込み可能なフォルダで使ってください。

初回は **1.0.0 / `v1.0.0`**、以後はmainへのpushごとに既存タグの最大版からpatchを増やします（`1.0.1`、`1.0.2`…）。一度のpushに複数コミットがあっても1版です。連続pushは直列化し、すでに新しいソースに含まれる古いイベントはスキップします。タグは採番済みのソースを指し、タグ注釈の `Source-Commit` に元のmainコミットを記録します。

採番用コミットはタグ側だけに保存し、mainへは自動コミットしません。mainの `package.json` は次回版の基準値で、公開済みの実際の版はReleaseとタグで確認します。minor / majorを上げる場合は、公開済みの最大版より大きい値を指定して変更をmainへ反映してください。

```sh
npm run version:set -- 1.1.0
npm run version:check
npm run test:release
```

`version:set` はpackage.json、npm lock、Cargo workspace、Cargo lockのアプリ版を同期します。Tauriはpackage.jsonを参照します。依存ライブラリの版は変更しません。CIはNode.js 24とRust 1.97.0を使い、ActionもコミットSHAで固定しています。

初回はこれらのファイルをコミットしてmainへpushしてください。Actionsが有効であれば、標準の `GITHUB_TOKEN` だけでタグとReleaseを作成します。PATや署名用secretの登録は不要です。リポジトリ／組織のポリシーでActionsの書込みが禁止されている場合は、その制限の確認が必要です。

ビルドが失敗したら、原因を確認してActionsの同じ実行を **Re-run all jobs** で再実行します。同じソースは同じタグ・下書きを再利用し、版を重複発行しません。公開済みの版は変更せずスキップします。ソースを修正してmainへpushした場合は次のpatchになり、失敗した下書き・タグは残ります。古い下書きを後から公開しても、新しい版のLatest表示は巻き戻しません。手動実行はActionsのReleaseからmainを選択します。

macOSはad-hoc署名のみでApple公証はなく、Windowsの証明書署名も未設定です。OSによる警告・起動制限が発生する場合があります。SVN・OpenSSHは同梱せず、利用する実行先へ別途用意してください。自動アップデート配信は含みません。

## 現時点の制約

- ユーザーの指示により、Macで検証して開発を継続します。Windows／Linux SSHの実機受入は後日確認とし、対応確認済みとはしません。
- checkout/clone、ファイル編集、パスワード／MFA入力は未実装です。
- Git機能は今回の変更で追加しましたが、Git実操作・Tauri画面・Windows/Linux SSHの受入確認は未実施です。
- コピー／移動、externals、switched、sparse、ツリー／プロパティ競合を含む作業コピーでは書込みを停止し、外部CLIでの処理を案内します。
- 一度の書込みは最大200対象・パス合計16000バイト、確認用差分は最大16 MiBです。超過時は実行しません。
- 更新でテキスト競合が発生した場合は外部エディタで解決し、「解決済みにする」を使ってください。既存のリポジトリロックはコミットで解放しません。
- 履歴は100件ずつ、差分は最大2 MiBです。ディレクトリ選択時はそのディレクトリ自身のプロパティ差分を表示します。変更ファイルは個別に選びます。
- 履歴の変更パスが登録作業コピーの範囲外の場合は差分表示を拒否します。削除済みパスなど、CLIが差分を取得できない履歴はエラーを表示します。
- UTF-8で表現できるパスが対象です。UNC、ネットワーク共有、WSL内作業コピーは未検証です。
- 処理の中断、ペイン幅の保存、履歴のパス絞込み、ロック情報の詳細表示、特殊構成の網羅検証は後続です。

仕様は [docs/SPEC.md](docs/SPEC.md)、現在位置と再開手順は [ROADMAP.md](ROADMAP.md) を参照してください。
