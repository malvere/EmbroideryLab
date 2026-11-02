# Synthetic regression inputs

Created independently for EmbroideryLab, 2026-11-01. These are not Embird exports and contain no Embird-derived implementation.

- `synthetic_holes.png`: two separated colors; white hole inside an annulus.
- `synthetic_two_colors.jpg`: simple blue triangle and red rectangle on white; JPEG compression is intentional.
- `synthetic_curves.svg`: cubic and quadratic Bezier curves, even-odd hole, group transform.
- `synthetic_pages.pdf`: created independently on 2026-11-02; portrait red vector artwork on page 1 and a blue embedded raster on landscape page 2, both on white paper.

Nominal import width: 100 mm. Raster source DPI is not used. Generation should preserve separate colored areas and holes; visual inspection remains necessary. `tests/make_quality_fixtures.py` reproduces the rasters using Pillow.
