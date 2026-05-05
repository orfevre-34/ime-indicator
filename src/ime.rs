// IME のモードを表すドメイン型。実際のモード検出は WH_KEYBOARD_LL フックで
// IME 切替キーの押下を直接拾う方式に統一済み。
//
// 以前は ImmGetContext + AttachThreadInput でフォアグラウンドアプリの IME
// 状態を 100ms 周期でポーリングしていたが、AttachThreadInput は入力キューを
// 共有する副作用があり、相手アプリで変換中の IME を勝手に確定 / 取消させて
// 入力を壊すことがあったため廃止した。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeMode {
    /// IME OFF: 直接入力（Mac の "A" 相当）
    Alpha,
    /// IME ON かつ かな変換モード（Mac の "あ" 相当）
    Hiragana,
    /// IME ON かつ かな以外の変換モード（カタカナ・全角英数など）
    Other,
}
