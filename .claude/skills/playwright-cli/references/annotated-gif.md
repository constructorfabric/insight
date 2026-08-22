# Annotated GIF Walkthroughs

Build a step-by-step walkthrough from screenshots when a bug report needs to show a path rather than a moment. Use it when video is unavailable or unreliable (see [video-recording.md](./video-recording.md)) and whenever the destination is an issue tracker — GitHub renders a GIF inline with no player, no controls and no click to start.

A GIF also beats video for this job in two ways: each step holds long enough to read, and a caption bar can state the measurement that makes the step evidence.

## The shape

One screenshot per step, a caption bar burned into each frame, assembled with Pillow.

```python
from PIL import Image, ImageDraw, ImageFont

BAR = 62
FONTS = ("/System/Library/Fonts/Supplemental/Arial Bold.ttf",
         "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
         "DejaVuSans-Bold.ttf")

def bar_font(size=21):
    for path in FONTS:
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    return ImageFont.load_default()

def annotate(src, dst, step, caption, width=None):
    im = Image.open(src).convert("RGB")
    if width:
        w, h = im.size
        im = im.resize((width, int(h * width / w)), Image.LANCZOS)
    out = Image.new("RGB", (im.width, im.height + BAR), (18, 26, 35))
    out.paste(im, (0, 0))
    d = ImageDraw.Draw(out)
    f = bar_font()
    d.text((18, im.height + BAR // 2), step, font=f, fill=(224, 121, 111), anchor="lm")
    d.text((18 + d.textlength(step, font=f) + 14, im.height + BAR // 2),
           caption, font=f, fill=(232, 238, 244), anchor="lm")
    out.save(dst)

shots = ["step1.png", "step2.png", "step3.png"]
steps = [("Step 1", "what you did", 2600),
         ("Step 2", "what came back", 3200),
         ("Step 3", "what was stored", 4200)]

assert len(shots) == len(steps), "one caption per screenshot"

annotated = []
for i, (src, (step, caption, _)) in enumerate(zip(shots, steps)):
    annotate(src, f"frame{i}.png", step, caption)
    annotated.append(f"frame{i}.png")

frames = [Image.open(p).convert("P", palette=Image.ADAPTIVE, colors=128) for p in annotated]
frames[0].save("repro.gif", save_all=True, append_images=frames[1:],
               duration=[ms for *_, ms in steps], loop=0, optimize=True, disposal=2)
```

`annotate` keeps each screenshot at its natural size; pass `width=` only when the shots are far larger than the tracker needs and the defect is not about layout. The length assert is what stops a missing caption from silently dropping a step — `zip` would truncate to the shorter list — and taking the durations from `steps` keeps one per frame however many there are.

`disposal=2` clears each frame before the next. Without it, frames of differing size ghost onto each other.

## Making the frames comparable

Screenshots taken at different viewport widths have different pixel sizes, and a GIF needs one canvas. Paste each onto a common canvas at its natural size rather than scaling to fit — a 390 px shot stretched to 1280 px hides the very thing a responsive bug is about.

```python
canvas = Image.new("RGB", (1280, 820), (12, 18, 25))
canvas.paste(Image.open(shot).convert("RGB"), (0, 0))
```

## Pointing at the defect

Draw the highlight yourself from the element's own box, so it lands exactly where the reader should look:

```bash
playwright-cli eval "() => { const e = document.querySelector('<sel>'); const r = e.getBoundingClientRect();
  return [r.left, r.top, r.right, r.bottom].map(Math.round).join(','); }"
```

Then a rounded rectangle at that box. For an absence — a control that should be there and is not — keep the rectangle in place across frames and strike it through on the frames where the element is gone. The eye tracks one position instead of hunting an empty region.

## Timing

Give each frame long enough to read its caption: roughly 2.5 s for a simple step, 4 s for the frame carrying the result. A closing frame that states the outcome in text — what was typed, what was stored, what was lost — makes the GIF readable without the issue body around it.

## Size

Keep the palette at 128 colours, and frames near 1100–1300 px wide where the source shots are wider than that and the defect is not a layout one. A four-frame walkthrough lands around 500 KB, well inside what a tracker accepts inline.
