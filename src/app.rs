use std::time::{Duration, Instant};

use crate::ime::ImeMode;

const FADE_IN: Duration = Duration::from_millis(120);

/// 表示ライフサイクル。
///
/// インジケータは常駐表示。起動直後だけ薄くフェードインしてから現れ、その後は
/// `Visible` のまま不透明度 1.0 で出続ける。モード変化で再度フェードインしたい
/// ときは `restart_fade_in` で `FadeIn` に戻す（が、ちらつきを避けるため現在は
/// 呼ばず、内容と座標だけ即座に差し替える）。
#[derive(Debug, Clone, Copy)]
pub enum Phase {
    FadeIn { start: Instant },
    Visible,
}

pub struct App {
    pub last_mode: Option<ImeMode>,
    pub current_mode: ImeMode,
    pub phase: Phase,
    pub anchor: (i32, i32),
}

impl App {
    pub fn new() -> Self {
        Self {
            last_mode: None,
            current_mode: ImeMode::Alpha,
            phase: Phase::FadeIn {
                start: Instant::now(),
            },
            anchor: (0, 0),
        }
    }

    /// IME のモードが変わったとき。常駐表示なのでフェードはやり直さず、
    /// 表示中の文字と位置だけ即座に切り替える（チラつき防止）。
    pub fn on_mode_changed(&mut self, new_mode: ImeMode, anchor: (i32, i32)) {
        self.last_mode = Some(self.current_mode);
        self.current_mode = new_mode;
        self.anchor = anchor;
    }

    /// 現在の不透明度を返す。`FadeIn` 中は 0→1 に補間、それ以外は 1.0。
    pub fn current_opacity(&mut self) -> f32 {
        let now = Instant::now();
        match self.phase {
            Phase::FadeIn { start } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= FADE_IN {
                    self.phase = Phase::Visible;
                    1.0
                } else {
                    ease_out(elapsed.as_secs_f32() / FADE_IN.as_secs_f32())
                }
            }
            Phase::Visible => 1.0,
        }
    }

    pub fn is_animating(&self) -> bool {
        matches!(self.phase, Phase::FadeIn { .. })
    }
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}
