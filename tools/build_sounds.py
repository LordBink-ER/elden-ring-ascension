#!/usr/bin/env python3
"""0.7 "Wygląd" (2.3 "Impact": 26 sounds): synthesized interface and combat sounds (assets/sounds/*.wav).

Bells, chords and impacts built from additive partials, filtered noise and a
small Schroeder reverb. Deterministic (fixed seeds), 22050 Hz mono 16-bit,
peaks at -6 dBFS; the DLL scales them by [ui] sound_volume and plays them with
winmm PlaySound (src/sound.rs). Run: python3 tools/build_sounds.py
"""
from __future__ import annotations

import struct
from pathlib import Path

import numpy as np

PROJECT = Path(__file__).resolve().parents[1]
OUT = PROJECT / "assets/sounds"
RATE = 22050


def t_axis(seconds):
    return np.arange(int(seconds * RATE), dtype=np.float64) / RATE


def env(t, attack, decay):
    a = np.clip(t / max(attack, 1e-4), 0, 1)
    return a * np.exp(-np.maximum(t - attack, 0) / decay)


def bell(freq, seconds, decay=0.9, bright=1.0):
    t = t_axis(seconds)
    # inharmonic bell partials
    ratios = [(1.0, 1.0, 1.0), (2.0, 0.55, 0.7), (2.76, 0.35 * bright, 0.5), (5.4, 0.18 * bright, 0.3),
              (8.9, 0.08 * bright, 0.18), (0.5, 0.25, 1.4)]
    out = np.zeros_like(t)
    for ratio, amp, dk in ratios:
        out += amp * np.sin(2 * np.pi * freq * ratio * t) * env(t, 0.003, decay * dk)
    return out


def noise(seconds, seed):
    return np.random.default_rng(seed).standard_normal(int(seconds * RATE))


def lowpass(x, cutoff):
    a = np.exp(-2 * np.pi * cutoff / RATE)
    y = np.zeros_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc = (1 - a) * v + a * acc
        y[i] = acc
    return y


def sweep_filter(x, start, end):
    """One-pole low pass whose cutoff glides from start to end Hz."""
    cut = np.geomspace(start, end, len(x))
    a = np.exp(-2 * np.pi * cut / RATE)
    y = np.zeros_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc = (1 - a[i]) * v + a[i] * acc
        y[i] = acc
    return y


def reverb(x, mix=0.3, tail=1.2):
    """Schroeder: four combs into two allpasses."""
    n = len(x) + int(tail * RATE)
    dry = np.concatenate([x, np.zeros(n - len(x))])
    wet = np.zeros(n)
    for delay_ms, gain in ((29.7, 0.80), (37.1, 0.78), (41.1, 0.76), (43.7, 0.74)):
        d = int(delay_ms * RATE / 1000)
        buf = np.copy(dry)
        for i in range(d, n):
            buf[i] += gain * buf[i - d]
        wet += buf
    wet /= 4
    for delay_ms, gain in ((5.0, 0.7), (1.7, 0.7)):
        d = int(delay_ms * RATE / 1000)
        y = np.zeros(n)
        for i in range(n):
            xd = wet[i - d] if i >= d else 0.0
            yd = y[i - d] if i >= d else 0.0
            y[i] = -gain * wet[i] + xd + gain * yd
        wet = y
    out = dry * (1 - mix) + wet * mix
    fade = np.ones(n)
    k = int(0.15 * RATE)
    fade[-k:] = np.linspace(1, 0, k)
    return out * fade


def place(base, sound, at):
    i = int(at * RATE)
    end = min(len(base), i + len(sound))
    base[i:end] += sound[: end - i]
    return base


def note(name):
    names = {"C": -9, "D": -7, "E": -5, "F": -4, "G": -2, "A": 0, "B": 2}
    semis = names[name[0]] + (1 if "#" in name else 0) + (int(name[-1]) - 4) * 12
    return 440.0 * 2 ** (semis / 12)


def deal():
    t = t_axis(0.45)
    x = noise(0.45, 1) * env(t, 0.06, 0.09)
    x = sweep_filter(x, 900, 5000)
    return reverb(x * 0.8, 0.2, 0.4)


def reveal_rare():
    x = bell(note("E6"), 1.4, 0.7) * 0.6 + bell(note("B6"), 1.4, 0.5, 0.6) * 0.3
    return reverb(x, 0.3, 1.0)


def reveal_epic():
    x = np.zeros(int(1.8 * RATE))
    place(x, bell(note("E5"), 1.6, 0.8) * 0.55, 0.0)
    place(x, bell(note("B5"), 1.5, 0.8) * 0.55, 0.12)
    place(x, bell(note("E6"), 1.3, 0.6, 0.6) * 0.35, 0.24)
    return reverb(x, 0.35, 1.2)


def chord(freqs, seconds, attack=0.25, decay=1.4, detune=0.004, seed=3):
    t = t_axis(seconds)
    rng = np.random.default_rng(seed)
    out = np.zeros_like(t)
    for f in freqs:
        for k in (-1, 0, 1):
            ff = f * (1 + k * detune)
            phase = rng.random() * 2 * np.pi
            # soft saw: a few harmonics
            for h, amp in ((1, 1.0), (2, 0.35), (3, 0.18), (4, 0.08)):
                out += amp / 3 * np.sin(2 * np.pi * ff * h * t + phase)
    return out * env(t, attack, decay)


def reveal_legendary():
    x = np.zeros(int(2.6 * RATE))
    place(x, chord([note("A3"), note("E4"), note("A4"), note("C#5")], 2.4, 0.18, 1.1) * 0.22, 0.0)
    place(x, bell(note("A5"), 1.8, 0.9) * 0.45, 0.05)
    place(x, bell(note("E6"), 1.6, 0.7, 0.7) * 0.30, 0.22)
    place(x, bell(note("A6"), 1.4, 0.6, 0.5) * 0.22, 0.38)
    return reverb(x, 0.4, 1.6)


def reveal_mythic():
    seconds = 3.2
    t = t_axis(seconds)
    drone = chord([note("D2"), note("A2"), note("D3")], seconds, 0.6, 2.2, 0.006, 5) * 0.3
    choir = chord([note("D4"), note("F4"), note("A4"), note("D5")], seconds, 0.45, 1.8, 0.008, 7) * 0.18
    shimmer = noise(seconds, 9) * env(t, 0.8, 0.9) * 0.08
    shimmer = sweep_filter(shimmer, 6000, 2500)
    x = drone + choir + shimmer
    place(x, bell(note("D6"), 2.0, 0.9) * 0.35, 0.35)
    place(x, bell(note("A6"), 1.8, 0.8, 0.6) * 0.22, 0.55)
    return reverb(x, 0.45, 2.0)


def choose():
    t = t_axis(0.9)
    thump = np.sin(2 * np.pi * 70 * t * (1 - 0.3 * t)) * env(t, 0.004, 0.12) * 0.9
    click = sweep_filter(noise(0.9, 11) * env(t, 0.001, 0.02), 4000, 800) * 0.6
    x = thump + click
    place(x, bell(note("A5"), 0.8, 0.5, 0.5) * 0.35, 0.03)
    return reverb(x, 0.25, 0.8)


def evolution():
    x = np.zeros(int(2.4 * RATE))
    for i, n in enumerate(["D5", "F#5", "A5", "D6", "F#6", "A6"]):
        place(x, bell(note(n), 1.4, 0.6, 0.6) * 0.32, i * 0.09)
    place(x, chord([note("D4"), note("A4"), note("D5")], 2.2, 0.3, 1.2) * 0.14, 0.0)
    return reverb(x, 0.4, 1.5)


# ---------------------------------------------------------------- 2.3 Impact
# Combat moments and banners. Layers: a transient (what you hit), a body
# (how heavy it was) and a tail (bells, choirs, reverb). Pitch glides use
# a running phase so sweeps stay smooth.

def highpass(x, cutoff):
    return x - lowpass(x, cutoff)


def bandpass(x, lo, hi):
    return lowpass(highpass(x, lo), hi)


def glide(f0, f1, seconds, curve=1.0):
    """Sine whose pitch slides from f0 to f1 (curve > 1 drops fast first)."""
    t = t_axis(seconds)
    k = (t / seconds) ** (1 / curve)
    freq = f0 + (f1 - f0) * k
    return np.sin(2 * np.pi * np.cumsum(freq) / RATE)


def drive(x, amount):
    """Soft saturation (more weight, no harsh clipping)."""
    return np.tanh(x * amount) / np.tanh(amount)


def saw(freqs, seconds, detune=0.003, seed=21):
    t = t_axis(seconds)
    rng = np.random.default_rng(seed)
    out = np.zeros_like(t)
    for f in freqs:
        for k in (-1, 1):
            ff = f * (1 + k * detune)
            phase = rng.random() * 2 * np.pi
            for h in range(1, min(30, int(9000 / ff)) + 1):
                out += np.sin(2 * np.pi * ff * h * t + phase * h) / h / 2
    return out


def impact(f0, f1, seconds, decay, amount=2.0):
    t = t_axis(seconds)
    return drive(glide(f0, f1, seconds, 2.5) * env(t, 0.002, decay), amount)


def grains(seconds, count, spread, lo, hi, seed, length=0.03):
    """Debris: short band-passed noise grains, denser at the start."""
    out = np.zeros(int(seconds * RATE))
    rng = np.random.default_rng(seed)
    for i in range(count):
        at = rng.exponential(spread)
        if at >= seconds - length:
            continue
        g = noise(length, seed * 100 + i)
        g = bandpass(g, lo, hi) * env(t_axis(length), 0.0005, length / 4)
        place(out, g * (0.9 ** i) * rng.uniform(0.5, 1.0), at)
    return out


def dodge():
    seconds = 0.9
    t = t_axis(seconds)
    x = np.zeros_like(t)
    # The dodge is confirmed after it happened: a short pre-swell only.
    swell = sweep_filter(noise(0.05, 13) * np.linspace(0, 1, int(0.05 * RATE)) ** 2, 400, 5000) * 0.8
    place(x, swell, 0.0)
    rush = sweep_filter(noise(0.5, 14) * env(t_axis(0.5), 0.004, 0.11), 5000, 700) * 0.9
    place(x, rush, 0.04)
    place(x, glide(85, 48, 0.4) * env(t_axis(0.4), 0.004, 0.14) * 0.5, 0.04)
    place(x, bell(note("B6"), 0.7, 0.35, 0.5) * 0.28, 0.06)
    place(x, bell(note("F#7"), 0.6, 0.3, 0.4) * 0.18, 0.10)
    return reverb(x, 0.35, 0.7)


def parry():
    seconds = 1.0
    t = t_axis(seconds)
    clang = np.zeros_like(t)
    drop = 1 - 0.01 * np.minimum(t / 0.2, 1)
    for ratio, amp, dk in ((1.0, 0.7, 0.32), (2.32, 0.5, 0.22), (3.86, 0.35, 0.15), (5.3, 0.25, 0.1), (7.1, 0.15, 0.06)):
        clang += amp * np.sin(2 * np.pi * 620 * ratio * drop * t) * env(t, 0.0008, dk)
    spark = highpass(noise(seconds, 17), 3000) * env(t, 0.0005, 0.02) * 0.8
    crackle = grains(seconds, 14, 0.05, 3000, 9000, 18, 0.012) * 0.5
    thump = glide(120, 70, seconds) * env(t, 0.002, 0.08) * 0.6
    return reverb((clang + spark + crackle + thump) * 0.8, 0.3, 0.7)


def critical():
    seconds = 1.1
    t = t_axis(seconds)
    x = impact(70, 38, seconds, 0.22, 2.5)
    x += bandpass(noise(seconds, 19), 300, 1800) * env(t, 0.001, 0.06) * 0.9
    place(x, bandpass(noise(0.3, 20), 250, 1400) * env(t_axis(0.3), 0.001, 0.05) * 0.45, 0.18)
    place(x, bell(note("E5"), 0.8, 0.35, 0.4) * 0.22, 0.01)
    return reverb(x, 0.28, 0.7)


def stance_break():
    seconds = 1.6
    t = t_axis(seconds)
    crack = highpass(noise(seconds, 31), 1500) * env(t, 0.0005, 0.035) * 1.0
    debris = grains(seconds, 28, 0.12, 2000, 6500, 32) * 0.7
    boom = impact(95, 32, seconds, 0.45, 2.2) * 1.1
    brass = sweep_filter(saw([note("D2"), note("A2"), note("D3")], seconds), 2200, 280) * env(t, 0.01, 0.5) * 0.35
    return reverb(crack + debris + boom + brass, 0.35, 1.2)


def backstab():
    seconds = 1.1
    t = t_axis(seconds)
    x = np.zeros_like(t)
    swish = sweep_filter(noise(0.14, 41) * env(t_axis(0.14), 0.03, 0.05), 6000, 900) * 0.8
    place(x, swish, 0.0)
    stab = glide(140, 60, 0.5) * env(t_axis(0.5), 0.001, 0.09) + bandpass(noise(0.5, 42), 500, 2500) * env(t_axis(0.5), 0.001, 0.04) * 0.7
    place(x, drive(stab, 1.8), 0.1)
    place(x, (bell(note("C4"), 1.0, 0.8, 0.3) + bell(note("C#4"), 1.0, 0.8, 0.3)) * 0.16, 0.12)
    x += lowpass(saw([55], seconds), 400) * env(t, 0.05, 0.5) * 0.15
    return reverb(x, 0.35, 0.9)


def execution():
    seconds = 2.0
    t = t_axis(seconds)
    x = impact(80, 30, seconds, 0.5, 2.4)
    x += bell(note("D3"), seconds, 1.6, 0.8) * 0.6 + bell(note("A3"), seconds, 1.2, 0.5) * 0.2
    x += chord([note("D3"), note("F3"), note("A3")], seconds, 0.15, 1.0) * 0.18
    return reverb(x, 0.45, 1.6)


def rampage():
    seconds = 1.8
    x = np.zeros(int(seconds * RATE))
    for at, amp in ((0.0, 0.7), (0.14, 0.8), (0.26, 1.0)):
        hit = impact(110, 48, 0.5, 0.16, 2.0) + lowpass(noise(0.5, int(at * 100) + 51), 1200) * env(t_axis(0.5), 0.001, 0.04) * 0.6
        place(x, hit * amp, at)
    roar_t = t_axis(1.4)
    roar = sweep_filter(saw([note("D2"), note("A2"), note("D3"), note("F3")], 1.4), 500, 2200) * env(roar_t, 0.02, 0.6)
    place(x, drive(roar, 2.0) * 0.4, 0.26)
    return reverb(x, 0.35, 1.2)


def block():
    seconds = 0.5
    t = t_axis(seconds)
    x = glide(160, 90, seconds) * env(t, 0.001, 0.07) * 0.9
    x += bandpass(noise(seconds, 61), 400, 1600) * env(t, 0.001, 0.03) * 0.6
    x += np.sin(2 * np.pi * 900 * t) * env(t, 0.001, 0.12) * 0.2
    return reverb(x, 0.2, 0.4)


def guard_counter():
    seconds = 1.0
    x = np.zeros(int(seconds * RATE))
    place(x, block()[: int(0.4 * RATE)] * 0.6, 0.0)
    place(x, sweep_filter(noise(0.18, 62) * env(t_axis(0.18), 0.04, 0.05), 1500, 6000) * 0.6, 0.08)
    place(x, parry()[: int(0.7 * RATE)] * 0.7, 0.2)
    return x


def status_blood():
    seconds = 0.9
    t = t_axis(seconds)
    x = lowpass(noise(seconds, 71), 900) * env(t, 0.002, 0.09) * 1.0
    x += bandpass(noise(seconds, 72), 1500, 5000) * env(t, 0.005, 0.25) * 0.35
    x += glide(90, 45, seconds) * env(t, 0.002, 0.12) * 0.7
    x += bell(note("C#5"), seconds, 0.4, 0.3) * 0.1
    return reverb(x, 0.25, 0.6)


def status_frost():
    seconds = 1.1
    t = t_axis(seconds)
    x = grains(seconds, 22, 0.08, 3000, 9000, 81, 0.01) * 0.9
    x += sweep_filter(noise(seconds, 82) * env(t, 0.02, 0.3), 7000, 2500) * 0.25
    place(x, bell(note("E7"), 0.9, 0.5, 0.3) * 0.25, 0.02)
    place(x, bell(note("B7"), 0.8, 0.45, 0.3) * 0.18, 0.06)
    return reverb(x, 0.4, 1.0)


def status_poison():
    seconds = 1.0
    t = t_axis(seconds)
    x = highpass(noise(seconds, 91), 2500) * env(t, 0.02, 0.35) * 0.25
    x += lowpass(noise(seconds, 92), 300) * env(t, 0.01, 0.3) * 0.3
    rng = np.random.default_rng(93)
    for i in range(14):
        at = rng.uniform(0, 0.6)
        f = rng.uniform(260, 420)
        blip = glide(f, f * 1.9, 0.045) * env(t_axis(0.045), 0.002, 0.015)
        place(x, blip * rng.uniform(0.25, 0.5), at)
    return reverb(x, 0.25, 0.6)


def status_sleep():
    seconds = 1.4
    x = np.zeros(int(seconds * RATE))
    for i, n in enumerate(["A5", "F5", "D5"]):
        place(x, lowpass(bell(note(n), 1.1, 0.7, 0.2), 3000) * 0.35, i * 0.14)
    x += chord([note("D4"), note("F4"), note("A4")], seconds, 0.2, 0.8) * 0.1
    return reverb(x, 0.5, 1.4)


def status_madness():
    seconds = 1.3
    t = t_axis(seconds)
    tremolo = 0.6 + 0.4 * np.sin(2 * np.pi * 12 * t)
    x = (bell(note("C5"), seconds, 0.8, 0.7) + bell(note("F#5"), seconds, 0.8, 0.7)) * 0.3 * tremolo
    x += bandpass(noise(seconds, 101), 800, 3000) * env(t, 0.003, 0.2) * 0.3
    x += glide(600, 1400, seconds) * env(t, 0.05, 0.4) * 0.15
    return reverb(x, 0.4, 1.0)


def status_blight():
    seconds = 1.5
    t = t_axis(seconds)
    x = chord([note("D2"), note("F2"), note("G#2")], seconds, 0.1, 0.9) * 0.3
    x += bandpass(noise(seconds, 111), 2000, 6000) * (0.6 + 0.4 * np.sin(2 * np.pi * 7 * t)) * env(t, 0.05, 0.6) * 0.15
    x += glide(60, 35, seconds) * env(t, 0.002, 0.3) * 0.6
    return reverb(x, 0.45, 1.4)


def elite_felled():
    seconds = 2.6
    x = impact(70, 30, seconds, 0.6, 2.2)
    x += chord([note("D3"), note("A3"), note("D4"), note("F#4")], seconds, 0.25, 1.6) * 0.25
    place(x, bell(note("D5"), 2.0, 1.0, 0.6) * 0.3, 0.1)
    place(x, bell(note("A5"), 1.8, 0.8, 0.5) * 0.2, 0.25)
    return reverb(x, 0.5, 2.0)


def path_bonus():
    seconds = 2.4
    t = t_axis(seconds)
    x = chord([note("A3"), note("E4"), note("A4")], seconds, 0.3, 1.4) * 0.12
    for i, n in enumerate(["A4", "C#5", "E5", "A5", "C#6"]):
        place(x, bell(note(n), 1.6, 0.7, 0.6) * 0.3, i * 0.09)
    x += highpass(noise(seconds, 121), 5000) * env(t, 0.3, 0.8) * 0.06
    return reverb(x, 0.45, 1.6)


def death_defied():
    seconds = 2.8
    x = np.zeros(int(seconds * RATE))
    for at, amp in ((0.0, 0.9), (0.28, 0.6)):
        place(x, glide(60, 40, 0.4) * env(t_axis(0.4), 0.002, 0.12) * amp, at)
    choir = chord([note("A3"), note("C#4"), note("E4"), note("A4")], 2.3, 0.5, 1.6) * 0.25
    place(x, choir, 0.45)
    place(x, bell(note("A5"), 1.8, 0.9, 0.6) * 0.35, 0.6)
    place(x, bell(note("E6"), 1.6, 0.8, 0.5) * 0.2, 0.8)
    return reverb(x, 0.5, 2.0)


def tide_rises():
    seconds = 3.0
    t = t_axis(seconds)
    horn = sweep_filter(saw([note("D1"), note("D2"), note("A2")], seconds), 150, 900) * env(t, 0.8, 1.5) * 0.45
    rumble = lowpass(noise(seconds, 131), 120) * env(t, 0.5, 1.2) * 0.35
    x = horn + rumble
    place(x, bell(note("D4"), 1.8, 1.0, 0.4) * 0.2, 0.9)
    return reverb(x, 0.5, 2.0)


def max_win():
    """3.4 Prismatic jackpot: a boom, a fast rising bell run, a bright choir
    swell, coin shimmer and a final chord. Mastered louder than every other
    sound (LOUD below)."""
    seconds = 4.2
    t = t_axis(seconds)
    x = impact(90, 40, seconds, 0.5, 2.6) * 0.9
    run = ["C5", "E5", "G5", "C6", "E6", "G6", "C7", "E6", "G6", "C7", "E7", "G7"]
    for i, n in enumerate(run):
        place(x, bell(note(n), 1.2, 0.6, 0.9) * 0.45, 0.05 + i * 0.07)
    choir = chord([note("C4"), note("E4"), note("G4"), note("C5"), note("E5")], 3.4, 0.35, 2.0) * 0.32
    place(x, choir, 0.25)
    rng = np.random.default_rng(141)
    for i in range(40):
        at = 0.3 + rng.exponential(0.6)
        if at < seconds - 0.4:
            f = rng.choice([note("C7"), note("E7"), note("G7"), note("C8")])
            place(x, bell(f, 0.35, 0.2, 0.6) * rng.uniform(0.08, 0.2), at)
    place(x, chord([note("C3"), note("G3"), note("C4"), note("E4"), note("G4")], 2.4, 0.05, 1.6) * 0.35, 1.0)
    place(x, bell(note("C6"), 2.8, 1.2, 1.0) * 0.5, 1.0)
    x += highpass(noise(seconds, 142), 6000) * env(t, 0.4, 1.2) * 0.05
    return reverb(drive(x, 1.4), 0.4, 1.6)


LOUD = {"max_win": 0.95}


SOUNDS = {
    "deal": deal, "reveal_rare": reveal_rare, "reveal_epic": reveal_epic, "reveal_legendary": reveal_legendary,
    "reveal_mythic": reveal_mythic, "choose": choose, "evolution": evolution, "dodge": dodge, "parry": parry,
    "critical": critical, "stance_break": stance_break, "backstab": backstab, "execution": execution,
    "rampage": rampage, "block": block, "guard_counter": guard_counter, "status_blood": status_blood,
    "status_frost": status_frost, "status_poison": status_poison, "status_sleep": status_sleep,
    "status_madness": status_madness, "status_blight": status_blight, "elite_felled": elite_felled,
    "path_bonus": path_bonus, "death_defied": death_defied, "tide_rises": tide_rises, "max_win": max_win,
}


def write_wav(path, x, level=0.5):
    peak = np.max(np.abs(x)) or 1.0
    data = np.clip(x / peak * level, -1, 1)
    pcm = (data * 32767).astype("<i2").tobytes()
    header = b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVE"
    header += b"fmt " + struct.pack("<IHHIIHH", 16, 1, 1, RATE, RATE * 2, 2, 16)
    header += b"data" + struct.pack("<I", len(pcm))
    path.write_bytes(header + pcm)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    total = 0
    for name, make in SOUNDS.items():
        path = OUT / f"{name}.wav"
        write_wav(path, make(), LOUD.get(name, 0.5))
        total += path.stat().st_size
    print(f"{len(SOUNDS)} sounds ({total / 1e3:.0f} kB)")


if __name__ == "__main__":
    main()
