// MVP 第一歩: フォアグラウンドウィンドウの IME 状態を 100ms 周期でポーリングし、
// モード変化をコンソールに出力する。オーバーレイは次フェーズで足す。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ime;

use std::thread;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

fn main() {
    // debug 実行ではコンソール出力を見たいので、現状はコンソールサブシステムのまま。
    // release では windows_subsystem = "windows" でコンソールを切る（=出力は捨てる）。
    let mut last: Option<ime::ImeMode> = None;
    println!("ime-indicator: polling IME state (Ctrl-C to stop)");

    loop {
        let current = ime::read_current_mode();
        if current != last {
            println!("IME: {:?} -> {:?}", last, current);
            last = current;
        }
        thread::sleep(POLL_INTERVAL);
    }
}
