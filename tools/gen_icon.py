# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "pillow>=10",
# ]
# ///
"""IME Indicator のアプリアイコンを生成する。

256/128/64/48/32/16 px の PNG を作って ICO にまとめる。
角丸ダーク背景に白い「あ」を太めの日本語フォントで描く、
Mac の IME インジケータ風のミニマル意匠。

実行:
    uv run tools/gen_icon.py
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
ASSETS.mkdir(exist_ok=True)

ICO_SIZES = [256, 128, 64, 48, 32, 24, 16]
PREVIEW_SIZE = 256

FONT_CANDIDATES = [
    r"C:\Windows\Fonts\YuGothB.ttc",
    r"C:\Windows\Fonts\NotoSansJP-VF.ttf",
    r"C:\Windows\Fonts\meiryob.ttc",
    r"C:\Windows\Fonts\meiryo.ttc",
]


def load_font(size_px: int) -> ImageFont.FreeTypeFont:
    """利用可能な日本語フォントを優先順に試して読み込む。"""
    last_err: Exception | None = None
    for path in FONT_CANDIDATES:
        try:
            return ImageFont.truetype(path, size=size_px)
        except OSError as e:
            last_err = e
            continue
    raise RuntimeError(f"No Japanese font found. Last error: {last_err}")


def draw_icon(size: int) -> Image.Image:
    """size×size の RGBA アイコンを 1 枚描く。"""
    # 高解像度で描いてから縮小すると小さいサイズでも輪郭が綺麗になる。
    scale = max(1, 4 if size >= 64 else 2)
    work = size * scale
    img = Image.new("RGBA", (work, work), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # 角丸矩形の背景: フラットダークグレー（過度なグラデは小さいサイズでノイズになる）。
    radius = int(work * 0.22)
    bg = (28, 28, 30, 240)
    draw.rounded_rectangle(
        [(0, 0), (work - 1, work - 1)], radius=radius, fill=bg
    )

    # 上端に薄いハイライトを 1px 入れて立体感を出す。
    highlight = Image.new("RGBA", (work, work), (0, 0, 0, 0))
    hd = ImageDraw.Draw(highlight)
    hd.rounded_rectangle(
        [(0, 0), (work - 1, work - 1)], radius=radius, outline=(255, 255, 255, 38), width=max(1, work // 128)
    )
    img.alpha_composite(highlight)

    # 「あ」を中央に配置。タスクバー/トレイで使われる小サイズではフォント比率を
    # 抑えて余白を確保しないと潰れて見える。
    glyph = "あ"
    glyph_ratio = 0.58 if size <= 32 else 0.62
    font = load_font(int(work * glyph_ratio))
    bbox = draw.textbbox((0, 0), glyph, font=font, anchor="lt")
    text_w = bbox[2] - bbox[0]
    text_h = bbox[3] - bbox[1]
    # bbox の左上オフセットも考慮して中央に。
    cx = work / 2 - (bbox[0] + text_w / 2)
    cy = work / 2 - (bbox[1] + text_h / 2)

    # ドロップシャドウ（小さくぼかす）。
    shadow = Image.new("RGBA", (work, work), (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow)
    sd.text((cx, cy + work * 0.012), glyph, font=font, fill=(0, 0, 0, 110))
    shadow = shadow.filter(ImageFilter.GaussianBlur(radius=work / 96))
    img.alpha_composite(shadow)

    # 本体グリフ。
    draw.text((cx, cy), glyph, font=font, fill=(245, 245, 247, 255))

    if scale > 1:
        img = img.resize((size, size), Image.LANCZOS)
    return img


def main() -> None:
    pngs: list[Image.Image] = []
    for s in ICO_SIZES:
        im = draw_icon(s)
        pngs.append(im)
        out = ASSETS / f"icon_{s}.png"
        im.save(out)
        print(f"wrote {out}")

    # ICO（Windows のリソースで最も大きい画像から順に並べる）。
    ico_path = ASSETS / "icon.ico"
    pngs[0].save(ico_path, format="ICO", sizes=[(s, s) for s in ICO_SIZES])
    print(f"wrote {ico_path}")

    # 確認用プレビュー（256x256 のままを別名で保存）。
    preview = draw_icon(PREVIEW_SIZE)
    preview_path = ASSETS / "icon_preview.png"
    preview.save(preview_path)
    print(f"wrote {preview_path}")


if __name__ == "__main__":
    main()
