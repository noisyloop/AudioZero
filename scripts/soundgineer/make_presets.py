#!/usr/bin/env python3
"""Generates extra Soundgineer presets as one JSON bundle (a list of preset files).

    python3 scripts/soundgineer/make_presets.py > crates/audiozero-engine/src/soundgineer/extra_presets.json

Each category is a recipe with a few knobs picked from a seeded random generator, so the output
is the same every time and the presets are all in the same file format as a saved preset
(see userpreset.rs). The engine's tests check that every one makes a clean, audible sound.
"""
import json, random, re, pathlib

DATA = pathlib.Path(__file__).resolve().parents[2] / "crates/audiozero-engine/src/soundgineer/data.rs"
LABELS = re.search(r"MOD_DEST_LABELS: \[&str; \d+\] = \[(.*?)\];", DATA.read_text()).group(1)
DESTS = [s.strip().strip('"') for s in LABELS.split('",')]
DESTS = [d.strip('"') for d in DESTS]
def dest(label): return DESTS.index(label)
SRC = {"env1": 1, "env2": 2, "lfo1": 7, "lfo2": 8, "velocity": 15, "keytrack": 16}

TABLES = ["Basic", "Sweep", "PWM", "Vocal", "Bell", "Digital"]
ADJ = ["Amber", "Basalt", "Cobalt", "Dusk", "Ember", "Flint", "Glacier", "Halo", "Indigo", "Jade",
       "Kite", "Lunar", "Moss", "Nova", "Onyx", "Pearl", "Quartz", "Rust", "Sable", "Tundra"]

def round_all(p): return {k: round(v, 3) for k, v in p.items()}

def mods(p, routes):
    for i, (src, label, depth) in enumerate(routes, 1):
        p[f"mod{i}.source"] = SRC[src]; p[f"mod{i}.dest"] = dest(label); p[f"mod{i}.depth"] = depth

def bass(r):
    p = {"osc1.wavetable": r.choice([0, 0, 2, 5]), "osc1.morph": r.uniform(0.0, 0.8), "osc1.transpose": -12,
         "osc1.level": r.uniform(0.6, 0.85), "sub.enabled": 1, "sub.level": r.uniform(0.4, 0.7), "sub.octave": -1,
         "filter1.type": r.choice([0, 1, 1]), "filter1.cutoff": r.uniform(500, 1200), "filter1.resonance": r.uniform(0.1, 0.45),
         "env1.attack": 0.003, "env1.decay": r.uniform(0.2, 0.8), "env1.sustain": r.uniform(0.6, 0.95), "env1.release": r.uniform(0.08, 0.25),
         "env2.attack": 0.003, "env2.decay": r.uniform(0.15, 0.5), "env2.sustain": r.uniform(0.0, 0.3)}
    if r.random() < 0.5: p.update({"osc1.unison": r.choice([2, 3, 5]), "osc1.detune": r.uniform(8, 30)})
    if r.random() < 0.4: p.update({"dist.type": r.choice([1, 3]), "dist.drive": r.uniform(0.2, 0.5)})
    mods(p, [("env2", "Filter 1 Cutoff", r.uniform(0.3, 0.6))])
    return p

def lead(r):
    p = {"osc1.wavetable": r.choice([0, 1, 2, 3, 5]), "osc1.morph": r.uniform(0.1, 0.8), "osc1.unison": r.choice([1, 3, 5, 7]),
         "osc1.detune": r.uniform(6, 22), "osc1.level": r.uniform(0.5, 0.7), "filter1.type": r.choice([0, 1]),
         "filter1.cutoff": r.uniform(2500, 9000), "filter1.resonance": r.uniform(0.1, 0.4),
         "env1.attack": r.uniform(0.002, 0.03), "env1.decay": 0.4, "env1.sustain": r.uniform(0.7, 0.95), "env1.release": r.uniform(0.15, 0.5),
         "lfo1.rate": r.uniform(4, 6.5), "lfo1.sync": 0, "delay.enabled": 1, "delay.mix": r.uniform(0.15, 0.3),
         "delay.feedback": r.uniform(0.25, 0.45), "reverb.enabled": 1, "reverb.mix": r.uniform(0.12, 0.25)}
    if r.random() < 0.5: p["master.polyphony"] = 1
    mods(p, [("lfo1", "Osc 1 Pitch", r.uniform(0.03, 0.08)), ("velocity", "Filter 1 Cutoff", 0.2)])
    return p

def pad(r):
    p = {"osc1.wavetable": r.choice([0, 1, 3, 4, 5]), "osc1.morph": r.uniform(0.1, 0.7), "osc1.unison": r.choice([5, 7]),
         "osc1.detune": r.uniform(10, 22), "osc1.spread": r.uniform(0.7, 1.0), "osc1.level": r.uniform(0.4, 0.55),
         "osc2.enabled": 1, "osc2.wavetable": r.choice([0, 1, 3]), "osc2.morph": r.uniform(0.1, 0.8), "osc2.unison": 5,
         "osc2.detune": r.uniform(8, 18), "osc2.transpose": r.choice([0, 12, -12]), "osc2.level": r.uniform(0.25, 0.4),
         "filter1.type": 0, "filter1.cutoff": r.uniform(1500, 5000), "filter1.resonance": r.uniform(0.05, 0.25),
         "env1.attack": r.uniform(0.4, 1.4), "env1.decay": 1.5, "env1.sustain": r.uniform(0.7, 0.9), "env1.release": r.uniform(1.2, 3.0),
         "lfo1.rate": r.uniform(0.08, 0.4), "lfo1.sync": 0, "chorus.enabled": 1, "chorus.mix": r.uniform(0.25, 0.5),
         "reverb.enabled": 1, "reverb.size": r.uniform(0.7, 0.92), "reverb.mix": r.uniform(0.3, 0.45)}
    mods(p, [("lfo1", "Osc 1 Morph", r.uniform(0.3, 0.6)), ("lfo2", "Filter 1 Cutoff", r.uniform(0.1, 0.25))])
    return p

def pluck(r):
    p = {"osc1.wavetable": r.choice([0, 1, 2, 4, 5]), "osc1.morph": r.uniform(0.05, 0.7), "osc1.level": r.uniform(0.55, 0.75),
         "osc1.unison": r.choice([1, 2, 3]), "osc1.detune": r.uniform(5, 14),
         "filter1.type": 1, "filter1.cutoff": r.uniform(700, 2500), "filter1.resonance": r.uniform(0.15, 0.4),
         "env1.attack": 0.002, "env1.decay": r.uniform(0.3, 0.8), "env1.sustain": 0.0, "env1.release": r.uniform(0.25, 0.6),
         "env2.attack": 0.001, "env2.decay": r.uniform(0.15, 0.4), "env2.sustain": 0.0,
         "delay.enabled": 1, "delay.mix": r.uniform(0.15, 0.3), "reverb.enabled": 1, "reverb.mix": r.uniform(0.15, 0.3)}
    mods(p, [("env2", "Filter 1 Cutoff", r.uniform(0.35, 0.6)), ("velocity", "Filter 1 Cutoff", 0.25)])
    return p

def keys(r):
    p = {"osc1.wavetable": r.choice([0, 2, 4]), "osc1.morph": r.uniform(0.05, 0.5), "osc1.level": r.uniform(0.5, 0.7),
         "osc2.enabled": 1, "osc2.wavetable": r.choice([0, 4]), "osc2.morph": r.uniform(0.0, 0.4), "osc2.transpose": r.choice([0, 12]),
         "osc2.level": r.uniform(0.2, 0.4), "filter1.type": 0, "filter1.cutoff": r.uniform(2500, 7000), "filter1.keytrack": r.uniform(0.3, 0.7),
         "env1.attack": 0.003, "env1.decay": r.uniform(0.6, 1.6), "env1.sustain": r.uniform(0.15, 0.5), "env1.release": r.uniform(0.2, 0.5),
         "chorus.enabled": 1, "chorus.mix": r.uniform(0.2, 0.4), "reverb.enabled": 1, "reverb.mix": r.uniform(0.15, 0.3)}
    mods(p, [("velocity", "Filter 1 Cutoff", 0.3), ("lfo1", "Osc 2 Morph", 0.15)])
    p.update({"lfo1.rate": r.uniform(0.3, 1.5), "lfo1.sync": 0})
    return p

def fx(r):
    p = {"osc1.wavetable": r.choice([1, 3, 4, 5]), "osc1.morph": r.uniform(0.1, 0.9), "osc1.unison": r.choice([3, 5, 7]),
         "osc1.detune": r.uniform(15, 40), "osc1.level": r.uniform(0.4, 0.55), "noise.enabled": 1, "noise.type": 1,
         "noise.level": r.uniform(0.1, 0.3), "filter1.type": r.choice([0, 4]), "filter1.cutoff": r.uniform(900, 4000),
         "filter1.resonance": r.uniform(0.2, 0.55), "env1.attack": r.uniform(0.5, 2.5), "env1.decay": 2.0, "env1.sustain": 0.8,
         "env1.release": r.uniform(1.0, 3.0), "lfo1.rate": r.uniform(0.05, 0.5), "lfo1.sync": 0,
         "phaser.enabled": 1, "phaser.mix": r.uniform(0.3, 0.6), "reverb.enabled": 1, "reverb.size": 0.9, "reverb.mix": r.uniform(0.3, 0.5)}
    mods(p, [("lfo1", "Filter 1 Cutoff", r.uniform(0.3, 0.7)), ("lfo1", "Osc 1 Morph", r.uniform(0.3, 0.7))])
    return p

CATEGORIES = [("Bass", bass), ("Leads", lead), ("Pads", pad), ("Plucks", pluck), ("Keys", keys), ("FX / other", fx)]
NOUN = {"Bass": ["Bass", "Sub", "Growl", "Low"], "Leads": ["Lead", "Solo", "Line", "Cry"],
        "Pads": ["Pad", "Wash", "Bed", "Haze"], "Plucks": ["Pluck", "Pick", "Pling", "Tick"],
        "Keys": ["Keys", "Tines", "Board", "Chord"], "FX / other": ["Drift", "Swell", "Texture", "Rise"]}

out = []
for ci, (cat, recipe) in enumerate(CATEGORIES):
    for i in range(12):
        r = random.Random(1000 * ci + i)
        params = round_all(recipe(r))
        # Names carry the category in the menu, so the name is just a colour and a noun.
        name = f"{ADJ[(i * 7 + ci * 3) % len(ADJ)]} {NOUN[cat][i % 4]}"
        out.append({"format": "audiozero-soundgineer-preset", "version": 1, "name": name,
                    "category": cat, "params": params})
# Names must be unique within the bundle (they are the menu entries).
seen = {}
for p in out:
    n = seen.get(p["name"], 0) + 1; seen[p["name"]] = n
    if n > 1: p["name"] += f" {n}"
print(json.dumps(out, indent=1))
