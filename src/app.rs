use std::time::{Duration, Instant};

use crate::ime::ImeMode;

const FADE_IN: Duration = Duration::from_millis(120);
const VISIBLE: Duration = Duration::from_millis(1500);
const FADE_OUT: Duration = Duration::from_millis(200);

/// 表示ライフサイクル。
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

    /// IME のモードが変わったときに呼ぶ。フェードイン開始。
    pub fn on_mode_changed(&mut self, new_mode: ImeMode, anchor: (i32, i32)) {
        self.last_mode = Some(self.current_mode);
        self.current_mode = new_mode;
        self.anchor = anchor;
        self.phase = Phase::FadeIn {
            start: Instant::now(),
        };
    }

    /// 16ms 周期で呼ぶ。フェード進行と相転移を扱い、現在の不透明度（0.0..=1.0）を返す。
    /// `None` を返したら描画不要（非表示状態）。
    pub fn tick(&mut self) -> Option<f32> {
        let now = Instant::now();
        match self.phase {
            Phase::Hidden => None,
            Phase::FadeIn { start } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= FADE_IN {
                    self.phase = Phase::Visible {
                        until: now + VISIBLE,
                    };
                    Some(1.0)
                } else {
                    Some(ease_out(elapsed.as_secs_f32() / FADE_IN.as_secs_f32()))
                }
            }
            Phase::Visible { until } => {
                if now >= until {
                    self.phase = Phase::FadeOut { start: now };
                    Some(1.0)
                } else {
                    Some(1.0)
                }
            }
            Phase::FadeOut { start } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= FADE_OUT {
                    self.phase = Phase::Hidden;
                    None
                } else {
                    Some(1.0 - ease_in(elapsed.as_secs_f32() / FADE_OUT.as_secs_f32()))
                }
            }
        }
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
