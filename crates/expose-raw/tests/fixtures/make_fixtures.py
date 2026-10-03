"""Makes the small test fixtures in this folder.

Run from this folder:

    conda run -n agent-env python make_fixtures.py

The fixtures come from a second, independent EXIF writer (Pillow), so that
the metadata tests do not only test the Rust test helpers.
"""

from PIL import Image
from PIL.TiffImagePlugin import IFDRational as R

# EXIF tag numbers.
MAKE = 0x010F
MODEL = 0x0110
ORIENTATION = 0x0112
EXIF_IFD = 0x8769
GPS_IFD = 0x8825
EXPOSURE_TIME = 0x829A
F_NUMBER = 0x829D
ISO = 0x8827
DATE_TIME_ORIGINAL = 0x9003
OFFSET_TIME_ORIGINAL = 0x9011
FOCAL_LENGTH = 0x920A
LENS_MODEL = 0xA434


def exif_jpeg(path: str) -> None:
    """A 32x16 JPEG: left half red, right half blue, orientation 3."""
    img = Image.new("RGB", (32, 16), (255, 0, 0))
    img.paste((0, 0, 255), (16, 0, 32, 16))
    exif = Image.Exif()
    exif[MAKE] = "FixtureMaker"
    exif[MODEL] = "Fixture One"
    exif[ORIENTATION] = 3
    sub = exif.get_ifd(EXIF_IFD)
    sub[EXPOSURE_TIME] = R(1, 250)
    sub[F_NUMBER] = R(28, 10)
    sub[ISO] = 800
    sub[DATE_TIME_ORIGINAL] = "2023:07:14 21:05:33"
    sub[OFFSET_TIME_ORIGINAL] = "-05:00"
    sub[FOCAL_LENGTH] = R(35, 1)
    sub[LENS_MODEL] = "Fixture 35mm F2.8"
    gps = exif.get_ifd(GPS_IFD)
    gps[1] = "S"
    gps[2] = (R(33, 1), R(52, 1), R(0, 1))
    gps[3] = "E"
    gps[4] = (R(151, 1), R(12, 1), R(36, 1))
    gps[5] = b"\x00"
    gps[6] = R(58, 1)
    img.save(path, "JPEG", quality=95, exif=exif.tobytes())


if __name__ == "__main__":
    exif_jpeg("pillow_exif.jpg")
