"""The intro films' sound: short synthesized effects placed on their motion cues.

Everything here is generated, so there are no sound files to license. The cue
times mirror the timings in film.html and the steps' pages; change one and change
the other.
"""

import wave

import numpy as np
from scipy.signal import butter, lfilter

RATE = 48000
# film.html's timings, in its "script" seconds: the opening, the length of each
# step, and how much slower than the script the opening, a step's name card and
# the rest of a step play.
OPEN, LENGTHS = 2.6, [4.2, 6.8, 5.6, 4.2, 6.6, 4.2]
CARD_END, OPEN_PACE, CARD_PACE, PACE = 1.9, 1.15, 1.6, 1.5
STARTS = [0.0, OPEN]
for length in LENGTHS:
    STARTS.append(STARTS[-1] + length)
END = STARTS[7] + 2.5
SPANS = [(0.0, OPEN, OPEN_PACE)]
for start, length in zip(STARTS[1:], LENGTHS):
    SPANS += [(start, start + CARD_END, CARD_PACE), (start + CARD_END, start + length, PACE)]
SPANS.append((STARTS[7], END, 1.0))
# When each thing is clicked, in script seconds into its step (film.html's CLICK).
CLICK = {"place": 2.0, "easy": 3.1, "easy2": 5.6, "turn": 4.4, "best": 3.05, "star": 2.0, "open": 2.6, "add": 3.3, "add2": 3.9, "pick": 4.8, "trash": 5.5, "apply": 2.55}


def real(script):
    """The moment in the finished film that shows `script` seconds of the script."""
    out = 0.0
    for start, end, pace in SPANS:
        if script <= end:
            return out + (max(script, start) - start) * pace
        out += (end - start) * pace
    return out


DURATION = real(END)


class Mix:
    """A film's sound track. Cues are placed at the film's own times, which `real`
    turns into seconds of the finished film."""

    def __init__(self, duration, real=lambda at: at):
        self.out = np.zeros((int(duration * RATE), 2))
        self.noise = np.random.default_rng(7)
        self.real = real

    def add(self, at, mono, pan=0.0):
        """Mixes `mono` in at `at`; `pan` runs -1 (left) to 1 (right)."""
        start = int(self.real(at) * RATE)
        mono = mono[: max(0, len(self.out) - start)]
        left, right = np.cos((pan + 1) * np.pi / 4), np.sin((pan + 1) * np.pi / 4)
        self.out[start : start + len(mono), 0] += mono * left
        self.out[start : start + len(mono), 1] += mono * right

    def glide(self, at, f0, f1, length, amp, decay=5.0, pan=0.0):
        """A sine sliding from f0 to f1 that dies away: pops, blips and thuds."""
        t = np.arange(int(length * RATE)) / RATE
        phase = 2 * np.pi * np.cumsum(f0 * (f1 / f0) ** (t / length)) / RATE
        attack = np.minimum(1, t / 0.004)
        self.add(at, amp * attack * np.exp(-decay * t / length) * np.sin(phase), pan)

    def tick(self, at, freq, amp=0.12, pan=0.0):
        self.glide(at, freq, freq * 0.97, 0.09, amp, 7.0, pan)

    def chime(self, at, freqs, amp=0.1, length=1.6):
        t = np.arange(int(length * RATE)) / RATE
        for n, freq in enumerate(freqs):
            tone = np.sin(2 * np.pi * freq * t) + 0.3 * np.sin(2 * np.pi * freq * 2.005 * t)
            self.add(at + 0.045 * n, amp * np.minimum(1, t / 0.006) * np.exp(-3.2 * t) * tone, (n - 1) * 0.3)

    def whoosh(self, at, length, amp=0.1, low=350, high=2600, sweep=1.0):
        """Filtered noise that swells and passes from one side to the other."""
        n = int(length * RATE)
        b, a = butter(2, [low, high], btype="band", fs=RATE)
        body = lfilter(b, a, self.noise.standard_normal(n)) * np.sin(np.pi * np.arange(n) / n) ** 2
        pan = np.linspace(-0.6 * sweep, 0.6 * sweep, n)
        start = int(self.real(at) * RATE)
        n = min(n, len(self.out) - start)
        self.out[start : start + n, 0] += amp * body[:n] * np.cos((pan[:n] + 1) * np.pi / 4)
        self.out[start : start + n, 1] += amp * body[:n] * np.sin((pan[:n] + 1) * np.pi / 4)

    def bell(self, at, freq, amp=0.1, length=1.5, pan=0.0):
        """A struck, glassy note: a sine whose brightness dies faster than its ring."""
        t = np.arange(int(length * RATE)) / RATE
        shimmer = 2.4 * np.exp(-7 * t) * np.sin(2 * np.pi * freq * 3.5 * t)
        self.add(at, amp * np.minimum(1, t / 0.003) * np.exp(-2.4 * t) * np.sin(2 * np.pi * freq * t + shimmer), pan)

    def shutter(self, at, amp=0.16):
        """Two quick mechanical clicks, like a camera's."""
        b, a = butter(2, [1500, 7000], btype="band", fs=RATE)
        for offset, level in ((0.0, 1.0), (0.085, 0.7)):
            n = int(0.03 * RATE)
            burst = lfilter(b, a, self.noise.standard_normal(n)) * np.exp(-np.arange(n) / (0.006 * RATE))
            self.add(at + offset, amp * level * burst)
            self.glide(at + offset, 320, 180, 0.04, amp * level * 0.6, 6)


    def click(self, at):
        self.tick(at, 1400, 0.12)
        self.glide(at, 240, 150, 0.1, 0.12, 5)

    def finish(self):
        """The track, faded out at its end and levelled."""
        out = self.out
        tail = int(0.35 * RATE)
        out[-tail:] *= np.linspace(1, 0, tail)[:, None]
        return np.clip(out * (0.5 / np.abs(out).max()), -1, 1)


C5, E5, G5, C6 = 523.25, 659.25, 783.99, 1046.5


def intro():
    """The first-run film (film.html)."""
    m = Mix(DURATION, real)
    click = m.click
    opening, organize, easy, rotation, best, cull, review, ending = STARTS

    # Opening: the name rises, the window is drawn, and the photos appear in it.
    m.whoosh(opening + 0.05, 1.0, 0.06)
    m.whoosh(opening + 1.15, 1.1, 0.06, 600, 3000, sweep=0)
    for i in range(12):
        m.tick(opening + 1.8 + 0.05 * (i % 4 + i // 4), 700 + 60 * (i % 4), 0.035, (i % 4 - 1.5) * 0.4)
    # Every step: its tab is pressed as the view leans in, and the photos rearrange.
    for n, start in enumerate(STARTS[1:7]):
        m.whoosh(start - 0.45, 0.9, 0.06, 500, 2800, sweep=0)
        click(start)
        m.whoosh(start + 0.3, 1.1, 0.075, sweep=-1 if n % 2 else 1)

    # Organize: Place is chosen, and twelve photos settle into their folders' rows.
    click(organize + CLICK["place"])
    for i in range(12):
        m.tick(organize + CLICK["place"] + 0.85 + 0.07 * i, [880, 988, 1175][i // 4], 0.09, (i % 4 - 1.5) * 0.3)

    # Easy Delete: a copy, then a blurry shot, each matched, marked, confirmed and gone.
    for at, press in ((1.9, CLICK["easy"]), (4.4, CLICK["easy2"])):
        m.glide(easy + at, 520, 700, 0.12, 0.09, 5)
        m.glide(easy + at + 0.5, 620, 470, 0.14, 0.12, 4, -0.4)
        m.glide(easy + at + 0.6, 620, 930, 0.14, 0.1, 5, 0.4)
        click(easy + press)
        m.glide(easy + press + 0.3, 190, 70, 0.26, 0.2, 4, -0.4)
    m.whoosh(easy + 3.6, 1.0, 0.05, sweep=-0.6)

    # Fix Rotation: the photo is checked, found sideways, corrected and confirmed.
    m.glide(rotation + 2.0, 900, 1350, 0.47, 0.03, 0.5)
    m.glide(rotation + 2.47, 1350, 900, 0.47, 0.03, 0.5)
    m.glide(rotation + 3.1, 620, 470, 0.14, 0.11, 4, -0.4)
    m.whoosh(rotation + 3.1, 1.0, 0.07, 500, 3200, sweep=0.8)
    m.glide(rotation + 3.8, 620, 930, 0.14, 0.09, 5, 0.4)
    click(rotation + CLICK["turn"])
    m.glide(rotation + CLICK["turn"] + 0.12, 700, 1050, 0.12, 0.1, 5, 0.4)

    # Pick Best: the scores rise, one is suggested, the best stays.
    m.glide(best + 1.85, 280, 560, 0.75, 0.03, 0.6)
    m.glide(best + 2.6, 700, 1050, 0.12, 0.09, 5, 0.4)
    click(best + CLICK["best"])
    m.whoosh(best + CLICK["best"] + 0.1, 0.8, 0.06, 400, 2400, sweep=-0.6)
    m.chime(best + CLICK["best"] + 0.7, [E5, C6], 0.08, 1.0)

    # Cull: four stars for one photo, then three are opened side by side, one is
    # picked out and sent to the Trash.
    click(cull + CLICK["star"])
    for i in range(4):
        m.tick(cull + CLICK["star"] + 0.03 + 0.06 * i, 880 * 2 ** (i / 6), 0.09)
    for n, name in enumerate(("open", "add", "add2")):
        click(cull + CLICK[name])
        m.whoosh(cull + CLICK[name] + 0.05, 0.6, 0.045, sweep=(n - 1) * 0.6)
        m.glide(cull + CLICK[name] + 0.5, 520 + 90 * n, 700 + 90 * n, 0.1, 0.06, 5, (n - 1) * 0.4)
    click(cull + CLICK["pick"])
    click(cull + CLICK["trash"])
    m.glide(cull + CLICK["trash"] + 0.02, 520, 330, 0.16, 0.11, 4)
    m.glide(cull + CLICK["trash"] + 0.4, 190, 70, 0.26, 0.18, 4)
    m.whoosh(cull + CLICK["trash"] + 0.45, 0.7, 0.04, sweep=0)

    # Review: the rows arrive, Apply is pressed, each row is done.
    for k in range(3):
        m.tick(review + 1.9 + 0.12 * k, 520 + 60 * k, 0.07)
    click(review + CLICK["apply"])
    for k, note in enumerate([C5, E5, G5]):
        m.chime(review + CLICK["apply"] + 0.5 + 0.2 * k, [note], 0.07, 0.8)

    # Ending: PhotoSort's own sound. A shutter, then four struck notes climbing
    # to rest as the name settles.
    m.whoosh(ending + 0.1, 0.9, 0.05, sweep=0)
    m.shutter(ending + 0.78)
    for n, note in enumerate([659.25, 880.0, 1108.73, 1318.51]):
        m.bell(ending + 0.95 + 0.085 * n, note, 0.1, 1.5, (n - 1.5) * 0.3)
    m.bell(ending + 0.95 + 0.085 * 3, 440.0, 0.06, 1.3)

    return m.finish()


def organize():
    """The Organize step's film (organize.html); AT is its AT."""
    AT = {"title": 1.3, "group": 4.9, "menu": 8.0, "date": 8.9, "menu2": 11.6, "place": 12.5, "noGps": 14.3, "hand": 15.9,
          "drag": 18.9, "drop": 20.0, "drag2": 20.8, "drop2": 21.9, "rename": 22.9, "pairs": 25.9, "fan": 28.5, "review": 32.7,
          "tab": 36.1, "end": 40.3}
    m = Mix(AT["end"] + 1.2)
    # The window is drawn and the import appears in it.
    m.whoosh(0.1, 1.0, 0.06, 600, 3000, sweep=0)
    for i in range(14):
        m.tick(0.85 + 0.05 * (i % 5 + i // 5), 700 + 60 * (i % 5), 0.035, (i % 5 - 2) * 0.3)
    # Every card: the window steps back as its words come up.
    for name in ("title", "group", "hand", "pairs", "review"):
        m.whoosh(AT[name], 0.9, 0.045, 400, 2200, sweep=0)
    # Grouping: the menu is opened and a grouping chosen, and the photos settle
    # into rows; by date, then by location.
    for menu, name, notes in (("menu", "date", (784, 880, 988, 1047)), ("menu2", "place", (880, 988, 1175, 1319))):
        m.whoosh(AT[menu] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
        m.click(AT[menu])
        m.glide(AT[menu] + 0.05, 420, 620, 0.12, 0.07, 5)
        m.click(AT[name])
        m.whoosh(AT[name] + 0.15, 1.2, 0.07, sweep=1 if name == "date" else -1)
        for i in range(14):
            m.tick(AT[name] + 0.85 + 0.05 * i, notes[min(i // 4, 3)], 0.08, (i % 4 - 1.5) * 0.3)
    # "No GPS" is pointed out.
    m.glide(AT["noGps"], 520, 700, 0.14, 0.08, 5)
    m.glide(AT["noGps"] + 0.95, 700, 520, 0.14, 0.05, 5)
    # Two photos are picked up, carried and set down.
    for press, drop, pan in ((AT["drag"], AT["drop"], -0.3), (AT["drag2"], AT["drop2"], 0.3)):
        m.click(press)
        m.glide(press + 0.05, 420, 620, 0.12, 0.08, 5)
        m.whoosh(press + 0.15, drop - press - 0.1, 0.05, 500, 2600, sweep=0.6)
        m.glide(drop, 300, 150, 0.14, 0.14, 5, pan)
        m.tick(drop + 0.03, 1175, 0.09, pan)
    # A folder is renamed: its name is selected, typed over and set.
    m.whoosh(AT["rename"] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
    m.click(AT["rename"])
    m.click(AT["rename"] + 0.2)
    for i in range(11):
        m.tick(AT["rename"] + 0.6 + 0.1 * i, 1700 + 90 * (i * 5 % 4), 0.05, (i % 3 - 1) * 0.2)
    m.chime(AT["rename"] + 2.1, [E5, C6], 0.07, 0.9)
    # A photo is held up, its files fan out, close up again and go back.
    m.whoosh(AT["fan"], 0.8, 0.06, sweep=0)
    for i in range(3):
        m.glide(AT["fan"] + 0.75 + 0.12 * i, 520 + 130 * i, 700 + 130 * i, 0.12, 0.09, 5, (i - 1) * 0.5)
    m.whoosh(AT["fan"] + 2.4, 0.7, 0.05, sweep=0)
    m.glide(AT["fan"] + 2.9, 700, 520, 0.12, 0.07, 5)
    m.whoosh(AT["fan"] + 3.0, 0.7, 0.05, sweep=-0.6)
    m.tick(AT["fan"] + 3.65, 988, 0.08)
    # Review: its tab is pressed, the photos step back and the moves are listed.
    m.whoosh(AT["tab"] - 0.45, 0.9, 0.06, 500, 2800, sweep=0)
    m.click(AT["tab"])
    m.whoosh(AT["tab"] + 0.3, 1.1, 0.075)
    m.tick(AT["tab"] + 0.9, 520, 0.07)
    for i in range(3):
        m.tick(AT["tab"] + 1.3 + 0.12 * i, 580 + 60 * i, 0.06)
    m.chime(AT["tab"] + 1.9, [C5, G5], 0.07, 1.4)
    # The window is drawn away.
    m.whoosh(AT["end"] + 0.1, 1.0, 0.05, 600, 3000, sweep=0)
    return m.finish()


def easy_delete():
    """The Easy Delete step's film (easy-delete.html); AT is its AT."""
    AT = {"title": 1.3, "found": 4.9, "tour": 8.0, "tour2": 10.1, "scroll": 12.0, "tour3": 12.6, "accept": 14.9, "press": 18.4,
          "last": 20.5, "flip": 24.0, "review": 25.9, "tab": 29.5, "end": 33.7}
    m = Mix(AT["end"] + 1.2)
    # The window is drawn and the finds appear in it.
    m.whoosh(0.1, 1.0, 0.06, 600, 3000, sweep=0)
    for i in range(8):
        m.tick(0.9 + 0.05 * i, 700 + 60 * (i % 4), 0.035, (i % 4 - 1.5) * 0.3)
    for name in ("title", "found", "accept", "last", "review"):
        m.whoosh(AT[name], 0.9, 0.045, 400, 2200, sweep=0)
    # Each kind of find is leaned in on and outlined; the grid scrolls to the last.
    for n, name in enumerate(("tour", "tour2", "tour3")):
        m.whoosh(AT[name] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
        m.glide(AT[name] + 0.1, 520 + 90 * n, 700 + 90 * n, 0.14, 0.08, 5)
    m.whoosh(AT["scroll"], 0.9, 0.05, 400, 2400, sweep=0)
    # Every suggestion is accepted: each photo going to Trash is marked in turn.
    m.click(AT["press"])
    for i in range(7):
        m.glide(AT["press"] + 0.2 + 0.08 * i, 620 - 20 * i, 440 - 20 * i, 0.12, 0.09, 4, (i % 3 - 1) * 0.4)
    m.glide(AT["press"] + 0.9, 190, 70, 0.26, 0.14, 4)
    # One is switched back to Keep.
    m.whoosh(AT["flip"] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
    m.click(AT["flip"])
    m.glide(AT["flip"] + 0.06, 620, 930, 0.14, 0.1, 5)
    # Review: its tab is pressed, the photos step back and what goes is listed.
    m.whoosh(AT["tab"] - 0.45, 0.9, 0.06, 500, 2800, sweep=0)
    m.click(AT["tab"])
    m.whoosh(AT["tab"] + 0.3, 1.1, 0.075)
    m.tick(AT["tab"] + 0.9, 520, 0.07)
    for i in range(3):
        m.tick(AT["tab"] + 1.3 + 0.12 * i, 580 + 60 * i, 0.06)
    m.chime(AT["tab"] + 1.9, [C5, G5], 0.07, 1.4)
    m.whoosh(AT["end"] + 0.1, 1.0, 0.05, 600, 3000, sweep=0)
    return m.finish()


def fix_rotation():
    """The Fix Rotation step's film (fix-rotation.html); AT is its AT."""
    AT = {"title": 1.3, "shown": 4.9, "tour": 8.0, "use": 10.2, "accept": 12.4, "press": 15.9, "unsure": 18.0, "tour2": 21.1,
          "turn": 22.9, "review": 25.2, "tab": 28.8, "end": 33.0}
    m = Mix(AT["end"] + 1.2)
    # The window is drawn and the photos appear in it.
    m.whoosh(0.1, 1.0, 0.06, 600, 3000, sweep=0)
    for i in range(8):
        m.tick(0.9 + 0.05 * i, 700 + 60 * (i % 4), 0.035, (i % 4 - 1.5) * 0.3)
    for name in ("title", "shown", "accept", "unsure", "review"):
        m.whoosh(AT[name], 0.9, 0.045, 400, 2200, sweep=0)
    # One card is leaned in on and its suggestion used.
    m.whoosh(AT["tour"] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
    m.glide(AT["tour"] + 0.1, 520, 700, 0.14, 0.08, 5)
    m.click(AT["use"])
    m.glide(AT["use"] + 0.06, 560, 840, 0.16, 0.1, 5)
    # Every sure suggestion is accepted: the other two are staged in turn.
    m.click(AT["press"])
    for i in range(2):
        m.glide(AT["press"] + 0.3 + 0.12 * i, 600 + 60 * i, 880 + 60 * i, 0.16, 0.1, 5, (i - 0.5) * 0.5)
    # An unsure one is turned by hand: the photo swings round.
    m.whoosh(AT["tour2"] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
    m.glide(AT["tour2"] + 0.1, 610, 790, 0.14, 0.08, 5)
    m.click(AT["turn"])
    m.whoosh(AT["turn"] + 0.05, 0.6, 0.06, 500, 2600)
    m.glide(AT["turn"] + 0.5, 620, 930, 0.14, 0.1, 5)
    # Review: its tab is pressed, the photos step back and the turns are listed.
    m.whoosh(AT["tab"] - 0.45, 0.9, 0.06, 500, 2800, sweep=0)
    m.click(AT["tab"])
    m.whoosh(AT["tab"] + 0.3, 1.1, 0.075)
    m.tick(AT["tab"] + 0.9, 520, 0.07)
    for i in range(3):
        m.tick(AT["tab"] + 1.3 + 0.12 * i, 580 + 60 * i, 0.06)
    m.chime(AT["tab"] + 1.9, [C5, G5], 0.07, 1.4)
    m.whoosh(AT["end"] + 0.1, 1.0, 0.05, 600, 3000, sweep=0)
    return m.finish()


def pick_best():
    """The Pick Best step's film (pick-best.html); AT is its AT."""
    AT = {"title": 1.3, "similar": 4.9, "tour": 8.0, "tour2": 10.2, "choose": 12.4, "tour3": 15.5, "pick": 16.6, "accept": 18.6,
          "press": 22.1, "review": 24.4, "tab": 28.0, "end": 32.2}
    m = Mix(AT["end"] + 1.2)
    # The window is drawn and the groups appear in it.
    m.whoosh(0.1, 1.0, 0.06, 600, 3000, sweep=0)
    for i in range(8):
        m.tick(0.9 + 0.05 * i, 700 + 60 * (i % 4), 0.035, (i % 4 - 1.5) * 0.3)
    for name in ("title", "similar", "choose", "accept", "review"):
        m.whoosh(AT[name], 0.9, 0.045, 400, 2200, sweep=0)
    # Each group looked at is leaned in on and outlined.
    for n, name in enumerate(("tour", "tour2", "tour3")):
        m.whoosh(AT[name] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
        m.glide(AT[name] + 0.1, 520 + 90 * n, 700 + 90 * n, 0.14, 0.08, 5)
    # A photo is clicked: it is marked for the Trash.
    m.click(AT["pick"])
    m.glide(AT["pick"] + 0.06, 620, 440, 0.14, 0.1, 4)
    # The other groups are accepted: each photo going to the Trash is marked in turn.
    m.click(AT["press"])
    for i in range(6):
        m.glide(AT["press"] + 0.2 + 0.08 * i, 620 - 20 * i, 440 - 20 * i, 0.12, 0.09, 4, (i % 3 - 1) * 0.4)
    m.glide(AT["press"] + 0.9, 190, 70, 0.26, 0.14, 4)
    # Review: its tab is pressed, the photos step back and what goes is listed.
    m.whoosh(AT["tab"] - 0.45, 0.9, 0.06, 500, 2800, sweep=0)
    m.click(AT["tab"])
    m.whoosh(AT["tab"] + 0.3, 1.1, 0.075)
    m.tick(AT["tab"] + 0.9, 520, 0.07)
    for i in range(3):
        m.tick(AT["tab"] + 1.3 + 0.12 * i, 580 + 60 * i, 0.06)
    m.chime(AT["tab"] + 1.9, [C5, G5], 0.07, 1.4)
    m.whoosh(AT["end"] + 0.1, 1.0, 0.05, 600, 3000, sweep=0)
    return m.finish()


def cull():
    """The Cull step's film (cull.html); AT is its AT."""
    AT = {"title": 1.3, "one": 4.9, "next": 8.0, "next2": 8.9, "rate": 10.4, "star": 13.5, "trash": 15.3, "next3": 18.4, "bin": 19.4,
          "compare": 21.2, "sel": 24.3, "sel2": 25.0, "sel3": 25.7, "choose": 27.0, "bin2": 28.1, "review": 29.9, "tab": 33.5,
          "end": 37.7}
    m = Mix(AT["end"] + 1.2)
    # The window is drawn and the photos appear in it.
    m.whoosh(0.1, 1.0, 0.06, 600, 3000, sweep=0)
    for i in range(8):
        m.tick(0.9 + 0.05 * i, 700 + 60 * (i % 4), 0.035, (i % 4 - 1.5) * 0.3)
    for name in ("title", "one", "rate", "trash", "compare", "review"):
        m.whoosh(AT[name], 0.9, 0.045, 400, 2200, sweep=0)
    # The next photo, three times: each slides in.
    for name in ("next", "next2", "next3"):
        m.click(AT[name])
        m.whoosh(AT[name] + 0.02, 0.4, 0.045, 600, 2600)
    # Four stars light up one after another.
    m.click(AT["star"])
    for i in range(4):
        m.tick(AT["star"] + 0.08 + 0.09 * i, 780 + 110 * i, 0.06, (i - 1.5) * 0.2)
    # A photo is marked for the Trash.
    m.click(AT["bin"])
    m.glide(AT["bin"] + 0.06, 620, 440, 0.14, 0.1, 4)
    # Three photos are selected in the strip and come up side by side.
    for n, name in enumerate(("sel", "sel2", "sel3")):
        m.click(AT[name])
        m.glide(AT[name] + 0.05, 520 + 90 * n, 700 + 90 * n, 0.14, 0.08, 5, (n - 1) * 0.4)
    # One of them is chosen and marked.
    m.click(AT["choose"])
    m.click(AT["bin2"])
    m.glide(AT["bin2"] + 0.06, 620, 440, 0.14, 0.1, 4)
    # Review: its tab is pressed, the photos step back and the changes are listed.
    m.whoosh(AT["tab"] - 0.45, 0.9, 0.06, 500, 2800, sweep=0)
    m.click(AT["tab"])
    m.whoosh(AT["tab"] + 0.3, 1.1, 0.075)
    m.tick(AT["tab"] + 0.9, 520, 0.07)
    for i in range(3):
        m.tick(AT["tab"] + 1.3 + 0.12 * i, 580 + 60 * i, 0.06)
    m.chime(AT["tab"] + 1.9, [C5, G5], 0.07, 1.4)
    m.whoosh(AT["end"] + 0.1, 1.0, 0.05, 600, 3000, sweep=0)
    return m.finish()


def review():
    """The Review step's film (review.html); AT is its AT."""
    AT = {"title": 1.3, "list": 4.9, "tour": 8.0, "scroll": 10.0, "tour2": 10.9, "mind": 12.8, "hover": 15.9, "remove": 16.9,
          "apply": 18.9, "press": 22.4, "done": 24.0, "end": 28.0}
    m = Mix(AT["end"] + 1.2)
    # The window is drawn and the changes appear in it.
    m.whoosh(0.1, 1.0, 0.06, 600, 3000, sweep=0)
    for i in range(8):
        m.tick(0.9 + 0.05 * i, 700 + 60 * (i % 4), 0.035, (i % 4 - 1.5) * 0.3)
    for name in ("title", "list", "mind", "apply"):
        m.whoosh(AT[name], 0.9, 0.045, 400, 2200, sweep=0)
    # The list is looked through and scrolls.
    for n, name in enumerate(("tour", "tour2", "hover")):
        m.whoosh(AT[name] - 0.45, 0.8, 0.04, 500, 2800, sweep=0)
        m.glide(AT[name] + 0.1, 520 + 90 * n, 700 + 90 * n, 0.14, 0.08, 5)
    m.whoosh(AT["scroll"], 0.9, 0.05, 400, 2400, sweep=0)
    # One photo is taken off the list and steps back.
    m.click(AT["remove"])
    m.whoosh(AT["remove"] + 0.05, 0.8, 0.06)
    # Apply: the photos step back while the changes are made, then it is done.
    m.click(AT["press"])
    m.whoosh(AT["press"] + 0.2, 1.2, 0.075)
    for i in range(6):
        m.tick(AT["press"] + 0.55 + 0.14 * i, 560 + 50 * i, 0.05, (i % 3 - 1) * 0.4)
    m.chime(AT["done"], [C5, E5, G5], 0.08, 1.8)
    m.bell(AT["done"] + 0.25, G5 * 2, 0.04, 1.6)
    m.whoosh(AT["end"] + 0.1, 1.0, 0.05, 600, 3000, sweep=0)
    return m.finish()


FILMS = {"intro": intro, "organize": organize, "easy-delete": easy_delete, "fix-rotation": fix_rotation, "pick-best": pick_best,
         "cull": cull, "review": review}


def write(path, film="intro"):
    samples = (FILMS[film]() * 32767).astype("<i2")
    with wave.open(str(path), "wb") as f:
        f.setnchannels(2)
        f.setsampwidth(2)
        f.setframerate(RATE)
        f.writeframes(samples.tobytes())
