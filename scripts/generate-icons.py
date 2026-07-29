"""Generate Keyestra PNG and Windows ICO assets from the Expressive K geometry."""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "assets" / "icons"
PNG_DIR = ICON_DIR / "png"
TRAY_SIZES = (16, 20, 24, 32, 48, 64, 128, 256)
SCALE = 4

TEAL = "#146C6C"
WHITE = "#FFFFFF"
CORAL = "#D9672F"


def cubic(p0, p1, p2, p3, steps=20):
    points = []
    for index in range(steps + 1):
        t = index / steps
        u = 1 - t
        points.append(
            (
                u**3 * p0[0]
                + 3 * u**2 * t * p1[0]
                + 3 * u * t**2 * p2[0]
                + t**3 * p3[0],
                u**3 * p0[1]
                + 3 * u**2 * t * p1[1]
                + 3 * u * t**2 * p2[1]
                + t**3 * p3[1],
            )
        )
    return points


def scale_points(points, factor):
    return [(round(x * factor), round(y * factor)) for x, y in points]


def upper_arm():
    points = [(96, 124)]
    points += cubic((96, 124), (116, 105), (134, 83), (157, 62))[1:]
    points += cubic((157, 62), (163, 57), (169, 54), (177, 54))[1:]
    points += [(197, 54)]
    points += cubic((197, 54), (203, 54), (205, 58), (201, 64))[1:]
    points += cubic((201, 64), (182, 93), (150, 114), (102, 130))[1:]
    return points


def lower_arm():
    points = [(99, 128)]
    points += cubic((99, 128), (132, 140), (164, 164), (194, 195))[1:]
    points += cubic((194, 195), (199, 201), (196, 205), (190, 205))[1:]
    points += [(163, 205)]
    points += cubic((163, 205), (157, 205), (154, 202), (151, 197))[1:]
    points += cubic((151, 197), (137, 174), (119, 153), (96, 138))[1:]
    return points


def render_master(include_node):
    canvas = 256 * SCALE
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle(
        (8 * SCALE, 8 * SCALE, 248 * SCALE, 248 * SCALE),
        radius=48 * SCALE,
        fill=TEAL,
    )
    draw.rounded_rectangle(
        (61 * SCALE, 46 * SCALE, 103 * SCALE, 210 * SCALE),
        radius=20 * SCALE,
        fill=WHITE,
    )
    draw.polygon(scale_points(upper_arm(), SCALE), fill=WHITE)
    draw.polygon(scale_points(lower_arm(), SCALE), fill=WHITE)
    if include_node:
        draw.ellipse(
            (
                93 * SCALE,
                121 * SCALE,
                109 * SCALE,
                137 * SCALE,
            ),
            fill=CORAL,
        )
    return image


def resize(master, size):
    return master.resize((size, size), Image.Resampling.LANCZOS)


def validate(image, size):
    assert image.mode == "RGBA"
    assert image.size == (size, size)
    # Lanczos can leave a single alpha level at a corner on some target sizes.
    assert image.getpixel((0, 0))[3] <= 1
    assert image.getpixel((size // 2, size // 2))[3] == 255


def main():
    PNG_DIR.mkdir(parents=True, exist_ok=True)
    app_master = render_master(include_node=True)
    tray_master = render_master(include_node=False)

    app_path = PNG_DIR / "keyestra-app-256.png"
    app_image = resize(app_master, 256)
    validate(app_image, 256)
    app_image.save(app_path, optimize=True)

    tray_images = []
    for size in TRAY_SIZES:
        image = resize(tray_master, size)
        validate(image, size)
        image.save(PNG_DIR / f"keyestra-tray-{size}.png", optimize=True)
        tray_images.append(image)

    ico_path = ICON_DIR / "keyestra.ico"
    tray_images[-1].save(
        ico_path,
        format="ICO",
        append_images=tray_images[:-1],
        sizes=[(size, size) for size in TRAY_SIZES],
        bitmap_format="png",
    )

    with Image.open(ico_path) as icon:
        frames = sorted(set(icon.info.get("sizes", ())))
    expected = {(size, size) for size in TRAY_SIZES}
    if set(frames) != expected:
        raise RuntimeError(f"ICO sizes differ: expected {sorted(expected)}, got {frames}")

    rgba_path = ICON_DIR / "keyestra-tray-32.rgba"
    rgba_path.write_bytes(resize(tray_master, 32).tobytes())
    if rgba_path.stat().st_size != 32 * 32 * 4:
        raise RuntimeError("Tray RGBA payload has the wrong byte length")

    print(f"Generated {app_path.relative_to(ROOT)}")
    print(f"Generated {len(tray_images)} tray PNGs: {', '.join(map(str, TRAY_SIZES))} px")
    print(f"Generated {ico_path.relative_to(ROOT)} with sizes {frames}")
    print(f"Generated {rgba_path.relative_to(ROOT)} for the runtime tray icon")


if __name__ == "__main__":
    main()
