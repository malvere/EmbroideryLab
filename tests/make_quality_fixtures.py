"""Create original synthetic fixtures; these are NOT Embird reference exports."""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parent/"fixtures"
ROOT.mkdir(parents=True,exist_ok=True)

# 100 mm square raster coordinates; red annulus and disconnected blue box.
image=Image.new("RGB",(400,400),"white")
draw=ImageDraw.Draw(image)
draw.ellipse((35,35,245,245),fill=(196,52,65))
draw.ellipse((90,90,190,190),fill="white")
draw.rounded_rectangle((255,220,365,365),radius=20,fill=(18,126,160))
image.save(ROOT/"synthetic_holes.png")

image=Image.new("RGB",(400,240),"white")
draw=ImageDraw.Draw(image)
draw.polygon([(25,185),(100,25),(175,185)],fill=(44,112,184))
draw.rectangle((230,45,370,185),fill=(215,71,54))
image.save(ROOT/"synthetic_two_colors.jpg",quality=97,subsampling=0)

(ROOT/"synthetic_curves.svg").write_text('''<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="70mm" viewBox="0 0 100 70">
<path fill="#b53657" fill-rule="evenodd" d="M 10 35 C 10 5 50 5 50 35 C 50 65 10 65 10 35 Z M 23 35 C 23 22 37 22 37 35 C 37 48 23 48 23 35 Z"/>
<g transform="translate(62 15) rotate(8 12 20)"><path fill="#158e8a" d="M 0 10 C 0 -5 25 -5 25 10 L 25 37 Q 12 55 0 37 Z"/></g>
</svg>''',encoding="utf-8")

(ROOT/"README.md").write_text('''# Synthetic regression inputs

Created independently for EmbroideryLab, 2026-11-01. These are not Embird exports and contain no Embird-derived implementation.

- `synthetic_holes.png`: two separated colors; white hole inside an annulus.
- `synthetic_two_colors.jpg`: simple blue triangle and red rectangle on white; JPEG compression is intentional.
- `synthetic_curves.svg`: cubic and quadratic Bezier curves, even-odd hole, group transform.

Nominal import width: 100 mm. Raster source DPI is not used. Generation should preserve separate colored areas and holes; visual inspection remains necessary. `tests/make_quality_fixtures.py` reproduces the rasters using Pillow.
''',encoding="utf-8")
print(ROOT)
