// アプリアイコンを Windows のリソースとして exe に埋め込む。
// アイコンリソース ID は 1（main.rs 側で MAKEINTRESOURCE(1) で参照）。

fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target != "windows" {
        return;
    }
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed Windows resources");
}
