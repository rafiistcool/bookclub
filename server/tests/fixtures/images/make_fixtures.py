"""Writes the MED image fixtures (server/tests/fixtures/images/)."""
import sys
from pathlib import Path

from PIL import Image, ImageDraw
from PIL.TiffImagePlugin import IFDRational

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)

# A phone photo: GPS position, camera make/model, a comment, and orientation 6 (rotate 90° clockwise
# to display). Stored 120×80 landscape, displayed 80×120 portrait. The top half of the stored image
# is red, the bottom half blue, so after the rotation the left half is blue and the right half red.
photo = Image.new("RGB", (120, 80), (200, 30, 30))
ImageDraw.Draw(photo).rectangle((0, 40, 119, 79), fill=(30, 60, 200))
exif = Image.Exif()
exif[0x010F] = "BookclubTestCam"          # Make
exif[0x0110] = "GPS Phone 9000"           # Model
exif[0x0112] = 6                          # Orientation: rotate 90° CW
exif[0x0131] = "SecretEditor 1.0"         # Software
gps = {
    1: "N",                                                   # GPSLatitudeRef
    2: (IFDRational(52, 1), IFDRational(31, 1), IFDRational(1234, 100)),   # 52°31'12.34"
    3: "E",                                                   # GPSLongitudeRef
    4: (IFDRational(13, 1), IFDRational(24, 1), IFDRational(5678, 100)),   # 13°24'56.78"
}
exif[0x8825] = gps
photo.save(out / "gps.jpg", format="JPEG", quality=92, exif=exif.tobytes())

# A small file with too many pixels: 6000×6000, one bit per pixel.
Image.new("1", (6000, 6000), 0).save(out / "too_many_pixels.png", format="PNG", optimize=True)

# A transparent PNG: a green square on a fully transparent background.
clear = Image.new("RGBA", (40, 40), (0, 0, 0, 0))
ImageDraw.Draw(clear).rectangle((10, 10, 29, 29), fill=(20, 160, 60, 255))
clear.save(out / "transparent.png", format="PNG")

# An animated GIF: a red first frame, a blue second frame.
frames = [Image.new("RGB", (30, 30), (220, 20, 20)), Image.new("RGB", (30, 30), (20, 20, 220))]
frames[0].save(out / "animated.gif", format="GIF", save_all=True, append_images=frames[1:], duration=100, loop=0)

# A truncated PNG: valid magic and header, no image data.
whole = (out / "transparent.png").read_bytes()
(out / "truncated.png").write_bytes(whole[:60])
