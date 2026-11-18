#!/usr/bin/env python3
"""Genera tests/data/sample_releases.xml: una muestra SINTÉTICA con el formato del dump de
releases de Discogs (mismas etiquetas y atributos), para los tests.

Todos los artistas, sellos y títulos son inventados. Cuando haya acceso al dump real, la muestra
se puede sustituir por una real con:

    cargo run --bin atenea-muestra -- discogs_AAAAMMDD_releases.xml.gz tests/data/sample_releases.xml 2000

Los tests solo dependen de los discos fijos del principio (ids 1–9), que se mantienen a mano
en tests/data/fixed_releases.xml.

Uso: python3 tools/gen_sample.py  (determinista: misma semilla, mismo archivo)
"""

import random
from pathlib import Path
from xml.sax.saxutils import escape, quoteattr

ROOT = Path(__file__).resolve().parent.parent
N = 2000

GENRES = {
    "Electronic": ["Techno", "Tech House", "Deep House", "House", "Minimal", "Acid House",
                   "Electro", "Trance", "Ambient", "Breakbeat", "Drum n Bass", "Dub Techno"],
    "Rock": ["Punk", "Post-Punk", "Indie Rock", "Psychedelic Rock", "Hard Rock", "Shoegaze"],
    "Jazz": ["Hard Bop", "Fusion", "Free Jazz", "Soul-Jazz", "Modal"],
    "Funk / Soul": ["Funk", "Soul", "Disco", "Boogie"],
    "Hip Hop": ["Boom Bap", "Instrumental", "Conscious"],
    "Reggae": ["Dub", "Roots Reggae", "Dancehall"],
}
COUNTRIES = ["Germany", "UK", "US", "Netherlands", "France", "Spain", "Italy", "Japan",
             "Belgium", "Sweden"]
SYL = ["ka", "lo", "mi", "ra", "ten", "vo", "zu", "dar", "nek", "sol", "bri", "mon", "tra",
       "vel", "qui", "rha", "dov", "len", "pax", "or"]
WORDS = ["Night", "Signal", "Motion", "Deep", "Frequency", "Ritual", "Orbit", "Static", "Echo",
         "Pulse", "Harbour", "Glass", "Velvet", "Machine", "Garden", "Shadow", "Drift", "Circuit",
         "Horizon", "Fever", "Silent", "Tide", "Neon", "Paper", "Iron", "Blue"]
VINYL_DESC = [['12"', '33 ⅓ RPM'], ['12"', '45 RPM', 'EP'], ['LP', 'Album'], ['7"', '45 RPM', 'Single'],
              ['12"', '33 ⅓ RPM', 'EP'], ['2xLP', 'Album']]
OTHER_FORMATS = [("CD", ["Album"]), ("File", ["MP3", "Album"]), ("Cassette", ["Album"])]


def name(rng, parts=2):
    return "".join(rng.choice(SYL) for _ in range(parts)).capitalize()


def title(rng):
    return " ".join(rng.choice(WORDS) for _ in range(rng.randint(1, 3)))


def release(rng, rid, artists_pool, labels_pool):
    genre = rng.choice(list(GENRES))
    genres = [genre] + ([rng.choice([g for g in GENRES if g != genre])] if rng.random() < 0.12 else [])
    styles = rng.sample(GENRES[genre], rng.randint(1, 2))
    n_art = 2 if rng.random() < 0.15 else 1
    arts = rng.sample(artists_pool, n_art)
    label_id, label_name = rng.choice(labels_pool)
    year = rng.randint(1968, 2026)
    month = rng.choice(["00", "03", "06", "09", "11"])
    if rng.random() < 0.8:
        fmt_name, desc = "Vinyl", rng.choice(VINYL_DESC)
        qty = 2 if desc[0].startswith("2x") else 1
    else:
        (fmt_name, desc), qty = rng.choice(OTHER_FORMATS), 1
    n_tracks = rng.randint(1, 8)
    sides = "AB" if n_tracks > 1 else "A"
    out = [f'<release id="{rid}" status="Accepted">']
    out.append("<artists>")
    for i, (aid, aname) in enumerate(arts):
        join = "&amp;" if i + 1 < len(arts) else ""
        out.append(f"<artist><id>{aid}</id><name>{escape(aname)}</name><anv></anv>"
                   f"<join>{join}</join><role></role><tracks></tracks></artist>")
    out.append("</artists>")
    out.append(f"<title>{escape(title(rng))}</title>")
    out.append(f'<labels><label name={quoteattr(label_name)} '
               f'catno={quoteattr(label_name[:3].upper() + " " + str(rng.randint(1, 250)).zfill(3))} '
               f'id="{label_id}"/></labels>')
    if rng.random() < 0.4:
        pid, pname = rng.choice(artists_pool)
        out.append(f"<extraartists><artist><id>{pid}</id><name>{escape(pname)}</name><anv></anv>"
                   f"<join></join><role>Producer</role><tracks></tracks></artist></extraartists>")
    descs = "".join(f"<description>{escape(d)}</description>" for d in desc)
    out.append(f'<formats><format name="{fmt_name}" qty="{qty}" text="">'
               f"<descriptions>{descs}</descriptions></format></formats>")
    out.append("<genres>" + "".join(f"<genre>{escape(g)}</genre>" for g in genres) + "</genres>")
    out.append("<styles>" + "".join(f"<style>{escape(s)}</style>" for s in styles) + "</styles>")
    out.append(f"<country>{rng.choice(COUNTRIES)}</country>")
    out.append(f"<released>{year}-{month}-00</released>" if month != "00" else f"<released>{year}</released>")
    if rng.random() < 0.3:
        out.append(f"<notes>{escape('Edición limitada. ' + title(rng) + '.')}</notes>")
    out.append("<data_quality>Needs Vote</data_quality>")
    out.append("<tracklist>")
    tracks = []
    for i in range(n_tracks):
        side = sides[0] if i < (n_tracks + 1) // 2 else sides[-1]
        pos = f"{side}{i + 1 if side == 'A' else i + 1 - (n_tracks + 1) // 2}"
        t = title(rng)
        dur = f"{rng.randint(2, 9)}:{rng.randint(0, 59):02d}" if rng.random() < 0.85 else ""
        tracks.append((t, dur))
        out.append(f"<track><position>{pos}</position><title>{escape(t)}</title>"
                   f"<duration>{dur}</duration></track>")
    out.append("</tracklist>")
    if rng.random() < 0.6:
        out.append("<videos>")
        for t, dur in tracks[: rng.randint(1, min(3, len(tracks)))]:
            vid = "".join(rng.choice("abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123456789_-") for _ in range(11))
            secs = int(dur.split(":")[0]) * 60 + int(dur.split(":")[1]) if dur else 300
            out.append(f'<video src="https://www.youtube.com/watch?v={vid}" duration="{secs}" embed="true">'
                       f"<title>{escape(arts[0][1] + ' - ' + t)}</title><description></description></video>")
        out.append("</videos>")
    out.append("</release>")
    return "".join(out)


def main():
    rng = random.Random(20261118)
    artists_pool = [(1000 + i, name(rng, rng.randint(2, 3)) + (" " + name(rng) if rng.random() < 0.5 else ""))
                    for i in range(400)]
    labels_pool = [(5000 + i, name(rng) + " " + rng.choice(["Records", "Recordings", "Music", "Tracks", "Audio"]))
                   for i in range(120)]
    fixed = (ROOT / "tests/data/fixed_releases.xml").read_text(encoding="utf-8").strip()
    lines = ['<?xml version="1.0" encoding="UTF-8"?>', "<releases>", fixed]
    for i in range(N - fixed.count("</release>")):
        lines.append(release(rng, 100 + i, artists_pool, labels_pool))
    lines.append("</releases>")
    out = ROOT / "tests/data/sample_releases.xml"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"{out} ({out.stat().st_size // 1024} KiB)")


if __name__ == "__main__":
    main()
