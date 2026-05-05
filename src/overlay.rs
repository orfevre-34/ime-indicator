use std::ffi::c_void;
use std::ptr;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Factory, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP,
    HDC, ReleaseDC, SelectObject,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmap, IWICImagingFactory,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{Result, w};

use crate::ime::ImeMode;

/// 表示するインジケータの論理サイズ（96 DPI 換算 / DIPs）。
const WIDTH_DIPS: f32 = 56.0;
const HEIGHT_DIPS: f32 = 36.0;
const CORNER_RADIUS_DIPS: f32 = 10.0;
const FONT_SIZE_DIPS: f32 = 22.0;

pub struct Overlay {
    hwnd: HWND,
    width_px: i32,
    height_px: i32,

    // GDI
    mem_dc: HDC,
    dib: HBITMAP,
    old_obj: windows::Win32::Graphics::Gdi::HGDIOBJ,
    bits: *mut c_void,

    // D2D / DWrite / WIC
    _wic_factory: IWICImagingFactory,
    _wic_bitmap: IWICBitmap,
    _d2d_factory: ID2D1Factory,
    rt: ID2D1RenderTarget,
    text_format: IDWriteTextFormat,
}

impl Overlay {
    pub fn new(hwnd: HWND, dpi_scale: f32) -> Result<Self> {
        let dpi_scale = dpi_scale.max(1.0);
        let width_px = (WIDTH_DIPS * dpi_scale).round() as i32;
        let height_px = (HEIGHT_DIPS * dpi_scale).round() as i32;

        unsafe {
            // 1. 画面 DC からメモリ DC + 32bpp DIB section を確保。
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width_px,
                    // 上下反転の DIB（top-down）にして座標を素直に扱う。
                    biHeight: -height_px,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };

            let mut bits: *mut c_void = ptr::null_mut();
            let dib = CreateDIBSection(
                Some(screen_dc),
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )?;
            let old_obj = SelectObject(mem_dc, dib.into());
            ReleaseDC(None, screen_dc);

            // 2. WIC bitmap を DIB のメモリに被せる（コピーしない）。
            let wic_factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

            let stride: u32 = (width_px * 4) as u32;
            let total: u32 = stride * height_px as u32;
            let buf: &[u8] = std::slice::from_raw_parts(bits as *const u8, total as usize);

            let wic_bitmap = wic_factory.CreateBitmapFromMemory(
                width_px as u32,
                height_px as u32,
                &GUID_WICPixelFormat32bppPBGRA,
                stride,
                buf,
            )?;

            // 3. Direct2D ファクトリと WIC bitmap 上のレンダーターゲット。
            let d2d_factory: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                // RT 内部の座標は DIP。WIC bitmap の物理 px と DIP のスケールを合わせる
                // ため、DPI を実 DPI で指定して描画は DIP で行えるようにする。
                dpiX: 96.0 * dpi_scale,
                dpiY: 96.0 * dpi_scale,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = d2d_factory.CreateWicBitmapRenderTarget(&wic_bitmap, &rt_props)?;

            // 4. DirectWrite text format。
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let text_format = dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                FONT_SIZE_DIPS,
                w!(""),
            )?;
            text_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            text_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            Ok(Self {
                hwnd,
                width_px,
                height_px,
                mem_dc,
                dib,
                old_obj,
                bits,
                _wic_factory: wic_factory,
                _wic_bitmap: wic_bitmap,
                _d2d_factory: d2d_factory,
                rt,
                text_format,
            })
        }
    }

    /// 指定座標にインジケータを再描画して表示更新する。
    pub fn render(&self, screen_x: i32, screen_y: i32, mode: ImeMode, opacity: f32) -> Result<()> {
        let opacity = opacity.clamp(0.0, 1.0);
        unsafe {
            self.rt.BeginDraw();

            // 完全透明でクリア。
            self.rt
                .Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

            // 背景ラウンド矩形: rgba(28, 28, 30, 0.85) を opacity で減衰。
            let bg = self.rt.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.110,
                    g: 0.110,
                    b: 0.118,
                    a: 0.85 * opacity,
                },
                None,
            )?;
            let rect = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: WIDTH_DIPS,
                bottom: HEIGHT_DIPS,
            };
            self.rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect,
                    radiusX: CORNER_RADIUS_DIPS,
                    radiusY: CORNER_RADIUS_DIPS,
                },
                &bg,
            );

            // テキスト。
            let glyph: &[u16] = match mode {
                ImeMode::Alpha => &[b'A' as u16],
                ImeMode::Hiragana => &[0x3042u16], // 'あ'
                ImeMode::Other => &[0x30ABu16],    // 'カ'
            };
            let fg: ID2D1SolidColorBrush = self.rt.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.95,
                    g: 0.95,
                    b: 0.97,
                    a: opacity,
                },
                None,
            )?;
            self.rt.DrawText(
                glyph,
                &self.text_format,
                &rect,
                &fg,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );

            self.rt.EndDraw(None, None)?;

            // UpdateLayeredWindow で位置 + 内容を反映。
            let pos = POINT {
                x: screen_x,
                y: screen_y,
            };
            let size = SIZE {
                cx: self.width_px,
                cy: self.height_px,
            };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };

            let screen_dc = GetDC(None);
            let result = UpdateLayeredWindow(
                self.hwnd,
                Some(screen_dc),
                Some(&pos),
                Some(&size),
                Some(self.mem_dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen_dc);
            result?;
        }
        Ok(())
    }

    pub fn size_px(&self) -> (i32, i32) {
        (self.width_px, self.height_px)
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            // DC から DIB を外して破棄。
            SelectObject(self.mem_dc, self.old_obj);
            let _ = DeleteObject(self.dib.into());
            let _ = DeleteDC(self.mem_dc);
            // _wic_bitmap は内部で `bits` を参照しているので、
            // ID2D1RenderTarget / IWICBitmap が drop されたあとに DIB を消すのが安全。
            // 構造体フィールドの drop 順は宣言順なので、rt → wic_bitmap → ここの DC/DIB の順。
            let _ = self.bits; // keep field used (suppress unused warning even if all paths use it)
        }
    }
}

