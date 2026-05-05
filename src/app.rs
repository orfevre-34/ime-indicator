use std::time::{Duration, Instant};

use crate::ime::ImeMode;

const FADE_IN: Duration = Duration::from_millis(120);
const VISIBLE: Duration = Duration::from_millis(1500);
const FADE_OUT: Duration = Duration::from_millis(200);

/// 表示ライフサイクル。
///
/// IME トリガーキーが押されるたびに `show_or_extend` が呼ばれて表示寿命が
/// 1.5 秒先まで伸びる。トリガーキーが 1.5 秒以上押されないとフェードアウトして
/// 消える。普通の文字キー等では表示は変化しない。
#[derive(Debug, Clone, Copy)]
pub enum Phase {
    Hidden,
    FadeIn { start: Instant },
    Visible { until: Instant },
    FadeOut { start: Instant },
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
            phase: Phase::Hidden,
            anchor: (0, 0),
        }
    }

    /// IME モード変化時。表示中の文字と座標を更新し、可視化を開始/延長する。
    pub fn on_mode_changed(&mut self, new_mode: ImeMode, anchor: (i32, i32)) {
        self.last_mode = Some(self.current_mode);
        self.current_mode = new_mode;
        self.anchor = anchor;
        self.show_or_extend();
    }

    fn show_or_extend(&mut self) {
        let now = Instant::now();
        match self.phase {
            Phase::Hidden | Phase::FadeOut { .. } => {
                self.phase = Phase::FadeIn { start: now };
            }
            Phase::FadeIn { .. } => {
                // フェードイン中は何もしない（完了後に Visible 期限が引かれる）。
            }
            Phase::Visible { .. } => {
                self.phase = Phase::Visible {
                    until: now + VISIBLE,
                };
            }
        }
    }

    /// 現在の不透明度を返し、必要なら相を進める。
    pub fn current_opacity(&mut self) -> f32 {
        let now = Instant::now();
        match self.phase {
            Phase::Hidden => 0.0,
            Phase::FadeIn { start } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= FADE_IN {
                    self.phase = Phase::Visible {
                        until: now + VISIBLE,
                    };
                    1.0
                } else {
                    ease_out(elapsed.as_secs_f32() / FADE_IN.as_secs_f32())
                }
            }
            Phase::Visible { until } => {
                if now >= until {
                    self.phase = Phase::FadeOut { start: now };
                    1.0
                } else {
                    1.0
                }
            }
            Phase::FadeOut { start } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= FADE_OUT {
                    self.phase = Phase::Hidden;
                    0.0
                } else {
                    1.0 - ease_in(elapsed.as_secs_f32() / FADE_OUT.as_secs_f32())
                }
            }
        }
    }

    pub fn is_hidden(&self) -> bool {
        matches!(self.phase, Phase::Hidden)
    }
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn ease_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t
}
