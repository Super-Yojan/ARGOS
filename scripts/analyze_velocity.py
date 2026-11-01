#!/usr/bin/env python3
"""Compare Terra's logged velocity estimate with OptiTrack ground truth.

Reads the phone's observability journal (events.db, read-only) and one Motive
CSV export per take. For each take it aligns the clocks by cross-correlating
the phone's yaw rate (raw gyro) with the mocap yaw rate, then reports bias,
scale, noise and its spectrum, latency, dropouts and spin behaviour against the
pass/fail criteria in the experiment design
(https://claude.ai/artifact/Q7oCNByu8PRHt8uhuFvVFc).

    uv run --with scipy --with matplotlib python analyze_velocity.py \
        --journal events.db --takes motive_csvs/ --out results/

Never writes to the journal. Requires numpy, scipy and matplotlib; writes a
Rerun recording per take when the rerun SDK is importable.
"""
import argparse
import csv
import json
import math
import sqlite3
import sys
from pathlib import Path

import numpy as np
from scipy import signal

FS = 100.0  # analysis grid, Hz: Terra's control tick
DT = 1.0 / FS
KINDS = ("control.summary", "control.output")


# ---------------------------------------------------------------- phone journal
def _iter_payloads(path):
    path = Path(path)
    if path.suffix == ".jsonl":
        with path.open() as f:
            for line in f:
                if line.strip():
                    yield json.loads(line)
        return
    with sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True) as db:
        tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        table = "observations" if "observations" in tables else "events" if "events" in tables else None
        if table is None:
            sys.exit(f"{path}: not a Terra journal (no observations/events table)")
        marks = ",".join("?" * len(KINDS))
        for kind_path in ("$.input.kind", "$.event.input.kind"):
            rows = db.execute(
                f"SELECT payload FROM {table} WHERE json_extract(payload,'{kind_path}') IN ({marks})", KINDS
            ).fetchall()
            if rows:
                for (p,) in rows:
                    yield json.loads(p)
                return


def load_phone(journal):
    """Return per-boot arrays of the controller's view, on the phone's monotonic clock."""
    boots = {}
    for row in _iter_payloads(journal):
        event = row.get("event", row)
        data = event.get("input", event)
        if data.get("kind") not in KINDS or not data.get("monotonic_ns"):
            continue
        ident = event.get("id", {})
        boot = (ident.get("producer_id", "?"), ident.get("boot_id", "?"))
        b = boots.setdefault(boot, {"summary": [], "output": []})
        a = data.get("attributes", {})
        t = data["monotonic_ns"] / 1e9
        if data["kind"] == "control.summary":
            b["summary"].append((t, a.get("estimated_forward", np.nan), a.get("estimated_yaw_rate", np.nan),
                                 1.0 if a.get("sensor_health") == "Ready" else 0.0))
        else:
            b["output"].append((t, a.get("target_forward", np.nan), a.get("left_effort", np.nan),
                                a.get("right_effort", np.nan)))
    if not boots:
        sys.exit("journal has no control.summary events; was Phone tracking running?")
    if len(boots) > 1:
        print(f"note: journal has {len(boots)} phone boots/producers; using the one with the most samples")
    best = max(boots.values(), key=lambda b: len(b["summary"]))
    s = np.array(sorted(best["summary"]), dtype=float)
    o = np.array(sorted(best["output"]), dtype=float) if best["output"] else np.zeros((0, 4))
    _, keep = np.unique(s[:, 0], return_index=True)
    s = s[keep]
    rate = 1.0 / np.median(np.diff(s[:, 0])) if len(s) > 2 else float("nan")
    print(f"phone: {len(s)} control samples, median rate {rate:.0f} Hz, "
          f"{(s[-1, 0] - s[0, 0]) / 60:.1f} min of log")
    return {"t": s[:, 0], "fwd": s[:, 1], "yaw": s[:, 2], "ready": s[:, 3], "rate": rate, "output": o}


# ---------------------------------------------------------------- Motive CSV
def load_motive(path, body, up):
    with open(path, newline="", encoding="utf-8-sig") as f:
        rows = list(csv.reader(f))
    meta = rows[0]
    info = {meta[i].strip(): meta[i + 1].strip() for i in range(0, len(meta) - 1, 2)}
    if info.get("Rotation Type", "Quaternion") != "Quaternion":
        sys.exit(f"{path}: export rotations as Quaternion, not {info.get('Rotation Type')}")
    units = info.get("Length Units", "Meters").lower()
    scale = {"meters": 1.0, "centimeters": 0.01, "millimeters": 0.001}.get(units, 1.0)

    def find(label):
        for i, r in enumerate(rows[:12]):
            if label in r[:2]:
                return i
        sys.exit(f"{path}: no '{label}' header row; is this a Motive tracking CSV?")

    ti, ni, fi = find("Type"), find("Name"), find("Frame")
    types, names, labels, kinds = rows[ti], rows[ni], rows[fi], rows[fi - 1]
    cols = {}
    for j in range(2, len(labels)):
        if j < len(types) and types[j].strip() == "Rigid Body" and names[j].strip() == body:
            cols[(kinds[j].strip(), labels[j].strip())] = j
    need = [("Rotation", a) for a in "XYZW"] + [("Position", a) for a in "XYZ"]
    if not all(k in cols for k in need):
        found = sorted({names[j] for j in range(2, len(types)) if j < len(names) and types[j].strip() == "Rigid Body"})
        sys.exit(f"{path}: rigid body '{body}' not found (found: {', '.join(found) or 'none'})")

    def num(x):
        try:
            return float(x)
        except ValueError:
            return np.nan

    data = np.array([[num(r[1])] + [num(r[cols[k]]) if cols[k] < len(r) else np.nan for k in need]
                     for r in rows[fi + 1:] if len(r) > 1 and r[1].strip()])
    t, q, p = data[:, 0], data[:, 1:5], data[:, 5:8] * scale
    # Planar coordinates with counter-clockwise-positive yaw seen from above.
    if up == "y":
        a, b = p[:, 0], -p[:, 2]
    else:
        a, b = p[:, 0], p[:, 1]
    x, y, z, w = q.T
    # Body x axis in world coordinates (first column of the rotation matrix).
    fx, fy, fz = 1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)
    fa, fb = (fx, -fz) if up == "y" else (fx, fy)
    return {"t": t, "a": a, "b": b, "psi": np.arctan2(fb, fa), "rate": 1.0 / np.nanmedian(np.diff(t))}


def _fill(t, x, max_gap):
    """Linearly fill NaN gaps up to max_gap seconds; return filled array and mask of long gaps."""
    bad = ~np.isfinite(x)
    if bad.all():
        return x, bad
    filled = x.copy()
    filled[bad] = np.interp(t[bad], t[~bad], x[~bad])
    long_gap = np.zeros_like(bad)
    edges = np.flatnonzero(np.diff(np.r_[0, bad.astype(int), 0]))
    for s, e in zip(edges[::2], edges[1::2]):
        if t[min(e, len(t) - 1)] - t[max(s - 1, 0)] > max_gap:
            long_gap[s:e] = True
    return filled, long_gap


def truth_on_grid(m, sg_window):
    t0, t1 = m["t"][0], m["t"][-1]
    grid = np.arange(t0, t1, DT)
    rate = m["rate"]
    win = max(5, int(round(sg_window * rate)) | 1)
    out, lost = {}, np.zeros(len(m["t"]), bool)
    for key in ("a", "b"):
        x, g = _fill(m["t"], m[key], 0.05)
        lost |= g
        out[key] = signal.savgol_filter(x, win, 3, deriv=1, delta=1 / rate)
    psi, g = _fill(m["t"], m["psi"], 0.05)
    psi = np.unwrap(psi)
    lost |= g
    yaw = signal.savgol_filter(psi, win, 3, deriv=1, delta=1 / rate)
    lost = np.convolve(lost, np.ones(win), "same") > 0
    res = {"t": grid}
    for k, v in (("va", out["a"]), ("vb", out["b"]), ("psi", psi), ("yaw", yaw)):
        res[k] = np.interp(grid, m["t"], v)
    res["valid"] = np.interp(grid, m["t"], (~lost).astype(float)) > 0.999
    return res


def heading_offset(tr):
    speed = np.hypot(tr["va"], tr["vb"])
    sel = tr["valid"] & (speed > 0.08) & (np.abs(tr["yaw"]) < 0.25)
    if sel.sum() < FS:  # less than 1 s of straight driving
        return None
    d = np.angle(np.exp(1j * (np.arctan2(tr["vb"][sel], tr["va"][sel]) - tr["psi"][sel])))
    # Mostly forward driving: fold reverse samples onto the forward direction.
    m = np.angle(np.mean(np.exp(2j * d))) / 2
    if np.mean(np.cos(d - m)) < 0:
        m += math.pi
    return float(np.angle(np.exp(1j * m)))


# ---------------------------------------------------------------- alignment
def _ncc_valid(sig, tmpl):
    tmpl = tmpl - tmpl.mean()
    num = signal.correlate(sig, tmpl, mode="valid", method="fft")
    win = np.ones(len(tmpl))
    s1 = signal.correlate(sig, win, mode="valid", method="fft")
    s2 = signal.correlate(sig * sig, win, mode="valid", method="fft")
    var = np.maximum(s2 - s1 * s1 / len(tmpl), 1e-12)
    return num / np.sqrt(var * np.sum(tmpl * tmpl))


def _parabolic(c, k):
    if 0 < k < len(c) - 1:
        den = c[k - 1] - 2 * c[k] + c[k + 1]
        if abs(den) > 1e-12:
            return k + 0.5 * (c[k - 1] - c[k + 1]) / den
    return float(k)


def align(phone, tr, search=None):
    """Offset c with phone_time = mocap_time + c, and the gyro sign."""
    pg = np.arange(phone["t"][0], phone["t"][-1], DT)
    py = np.interp(pg, phone["t"], np.nan_to_num(phone["yaw"]))
    ty = np.where(tr["valid"], tr["yaw"], 0.0)
    if len(ty) >= len(py):
        raise ValueError("take is longer than the phone log")
    c = _ncc_valid(py, ty)
    if search is not None:
        lo, hi = search
        idx = np.arange(len(c))
        offs = pg[0] + idx * DT - tr["t"][0]
        c = np.where((offs >= lo) & (offs <= hi), c, 0)
    k = int(np.argmax(np.abs(c)))
    sign = 1.0 if c[k] > 0 else -1.0
    kk = _parabolic(sign * c, k)
    offset = pg[0] + kk * DT - tr["t"][0]
    # Second-best peak at least 2 s away, as a confidence check.
    far = np.abs(np.arange(len(c)) - k) > 2 * FS
    second = float(np.max(np.abs(c[far]))) if far.any() else 0.0
    return offset, sign, float(abs(c[k])), second


# ---------------------------------------------------------------- metrics
def segments(mask, min_len):
    edges = np.flatnonzero(np.diff(np.r_[0, mask.astype(int), 0]))
    return [(s, e) for s, e in zip(edges[::2], edges[1::2]) if e - s >= min_len]


def latency(truth, est, valid, max_lag=0.5):
    if valid.sum() < 3 * FS or np.std(truth[valid]) < 0.03:
        return None, None
    lags = np.arange(-int(0.2 * FS), int(max_lag * FS) + 1)
    r = []
    for L in lags:
        if L >= 0:
            x, y, v = truth[: len(truth) - L], est[L:], valid[: len(valid) - L] & valid[L:]
        else:
            x, y, v = truth[-L:], est[: len(est) + L], valid[-L:] & valid[: len(valid) + L]
        r.append(np.corrcoef(x[v], y[v])[0, 1] if v.sum() > FS else np.nan)
    r = np.array(r)
    k = int(np.nanargmax(r))
    return float(lags[0] + _parabolic(r, k)) * DT, float(r[k])


def band_rms(f, p, lo, hi):
    sel = (f > lo) & (f <= hi)
    return float(math.sqrt(np.sum(p[sel]) * (f[1] - f[0]))) if sel.any() else float("nan")


def welch_segments(x, segs):
    """Length-weighted Welch PSD over steady segments, 10 s windows when they all fit."""
    long = [(s, e) for s, e in segs if e - s >= 4 * FS]
    if not long:
        return None
    n = int(10 * FS) if all(e - s >= 10 * FS for s, e in long) else int(4 * FS)
    acc, total, f = None, 0, None
    for s, e in long:
        f, p = signal.welch(x[s:e] - np.mean(x[s:e]), FS, window="hann", nperseg=n)
        acc = p * (e - s) if acc is None else acc + p * (e - s)
        total += e - s
    return f, acc / total


def scale_fit(means):
    """Weighted line through steady-segment means; needs two speeds 0.05 m/s apart."""
    m = [(v, e, n) for v, e, n in means if n >= 2 * FS]
    if len({round(v / 0.05) for v, _, _ in m}) < 2:
        return None
    v, e, n = (np.array(x) for x in zip(*m))
    slope, icpt = np.polyfit(v, e, 1, w=np.sqrt(n))
    return round(float(slope), 4), round(float(icpt), 4)


def analyze(name, phone, tr, offset, sign, args):
    tp = tr["t"] + offset
    inside = (tp >= phone["t"][0]) & (tp <= phone["t"][-1])
    est = np.interp(tp, phone["t"], phone["fwd"])
    est_yaw = sign * np.interp(tp, phone["t"], phone["yaw"])
    idx = np.clip(np.searchsorted(phone["t"], tp, "right") - 1, 0, len(phone["t"]) - 1)
    ready = (phone["ready"][idx] > 0.5) & inside
    # Gaps in the phone log longer than 3 samples count as not ready.
    gap = np.abs(phone["t"][idx] - tp) > 3.5 / phone["rate"]
    ready &= ~gap
    h = tr["psi"] + args.heading_offset
    fwd = tr["va"] * np.cos(h) + tr["vb"] * np.sin(h)
    acc = signal.savgol_filter(fwd, 21, 2, deriv=1, delta=DT)
    valid = tr["valid"] & ready
    err = est - fwd
    res = {"take": name, "duration_s": round(float(tr["t"][-1] - tr["t"][0]), 1),
           "clock_offset_s": round(offset, 4), "gyro_sign": int(sign)}

    # Dropouts over the whole take while the phone log covers it.
    nr = tr["valid"] & inside & ~ready
    drops = segments(nr, 1)
    res["not_ready_fraction"] = round(float(nr.sum() / max(1, (tr["valid"] & inside).sum())), 4)
    res["dropouts"] = len(drops)
    res["longest_dropout_s"] = round(max([(e - s) * DT for s, e in drops], default=0.0), 3)

    # Steady segments: little acceleration and turning for at least 1 s, edges trimmed.
    steady = valid & (np.abs(acc) < args.steady_accel) & (np.abs(tr["yaw"]) < 0.2)
    segs = [(s + int(0.2 * FS), e - int(0.2 * FS)) for s, e in segments(steady, int(1.0 * FS))]
    bins = {}
    for s, e in segs:
        v = float(np.mean(fwd[s:e]))
        key = round(round(v / 0.05) * 0.05, 2)
        bins.setdefault(key, []).append(err[s:e])
    res["speed_bins"] = []
    for key in sorted(bins):
        e = np.concatenate(bins[key])
        res["speed_bins"].append({"speed": key, "seconds": round(len(e) * DT, 1),
                                  "bias": round(float(np.mean(e)), 4), "std": round(float(np.std(e)), 4)})
    sel = np.zeros_like(valid)
    for s, e in segs:
        sel[s:e] = True
    # Scale from steady-segment means, so ramps and noise do not lever the fit.
    res["_segmeans"] = [(float(np.mean(fwd[s:e])), float(np.mean(est[s:e])), e - s) for s, e in segs]
    fit = scale_fit(res["_segmeans"])
    if fit:
        res["scale"], res["intercept"] = fit

    # In-band noise from the error spectrum (mocap noise is negligible at low
    # frequency); out-of-band noise from the estimate itself, so the mocap
    # differentiation noise at high frequency does not count against the phone.
    pe, pv = welch_segments(err, segs), welch_segments(est, segs)
    if pe and pv:
        res["noise_in_band"] = round(band_rms(*pe, 0, args.fc), 4)
        res["noise_out_band"] = round(band_rms(*pv, args.fc, FS / 2), 4)
        res["effort_chatter"] = round(args.kp * res["noise_out_band"], 4)
        res["_psd"] = pe

    lat, r = latency(fwd, est, valid)
    if lat is not None:
        res["latency_s"], res["latency_corr"] = round(lat, 4), round(r, 3)

    # Yaw rate: bias while still, scale and spurious forward speed while spinning.
    still = valid & (np.abs(tr["yaw"]) < 0.02) & (np.abs(fwd) < 0.01)
    if still.sum() > 2 * FS:
        res["yaw_bias"] = round(float(np.mean(est_yaw[still] - tr["yaw"][still])), 4)
    spin = valid & (np.abs(tr["yaw"]) > 0.3) & (np.abs(fwd) < 0.05)
    if spin.sum() > 2 * FS:
        res["spin_seconds"] = round(float(spin.sum() * DT), 1)
        res["spin_spurious_forward_rms"] = round(float(np.sqrt(np.mean(est[spin] ** 2))), 4)
    turning = valid & (np.abs(tr["yaw"]) > 0.2) & (np.abs(tr["yaw"]) < 1.5)  # excludes the sync jerks
    if turning.sum() > 2 * FS:
        res["yaw_scale"] = round(float(np.polyfit(tr["yaw"][turning], est_yaw[turning], 1)[0]), 4)
        res["turning_forward_bias"] = round(float(np.mean(est[turning] - fwd[turning])), 4)
    res["_series"] = {"t": tr["t"] - tr["t"][0], "fwd": fwd, "est": est, "yaw": tr["yaw"], "est_yaw": est_yaw,
                      "ready": ready, "valid": tr["valid"]}
    return res


def verdicts(r, args):
    v = {}
    if r["speed_bins"]:
        v["P1 bias"] = all(abs(b["bias"]) <= max(0.01, 0.03 * abs(b["speed"])) for b in r["speed_bins"])
    if "scale" in r and "intercept" in r:
        v["P2 scale"] = 0.97 <= r["scale"] <= 1.03
    if "noise_in_band" in r:
        v["P3 in-band noise"] = r["noise_in_band"] <= 0.02
        v["P4 effort chatter"] = r["effort_chatter"] <= 0.01
    if "latency_s" in r:
        budget = (100 - 45) / (360 * args.fc)
        v[f"P5 sensor delay < {budget * 1e3:.0f} ms budget"] = r["latency_s"] < budget
    if not r["take"].upper().startswith(("F", "G")):
        v["P7 dropouts"] = r["not_ready_fraction"] <= 0.01 and r["longest_dropout_s"] < 0.35
    if "spin_spurious_forward_rms" in r:
        v["P9 spin"] = r["spin_spurious_forward_rms"] <= 0.02 and abs(r.get("yaw_bias", 0)) <= 0.01
    return v


# ---------------------------------------------------------------- outputs
def plot(r, path, fc):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    s = r["_series"]
    fig, ax = plt.subplots(4, 1, figsize=(11, 11), constrained_layout=True)
    nr = ~s["ready"] & s["valid"]
    for a in ax[:3]:
        for st, en in segments(nr, 1):
            a.axvspan(s["t"][st], s["t"][min(en, len(s["t"]) - 1)], color="tab:red", alpha=0.15, lw=0)
    ax[0].plot(s["t"], s["fwd"], color="tab:orange", lw=1.4, label="OptiTrack")
    ax[0].plot(s["t"], s["est"], color="tab:blue", lw=1, label="phone estimate")
    ax[0].set_ylabel("forward speed (m/s)")
    ax[0].legend(loc="upper right")
    ax[0].set_title(f"{r['take']}  (red = estimate not Ready)")
    ax[1].plot(s["t"], s["yaw"], color="tab:orange", lw=1.4)
    ax[1].plot(s["t"], s["est_yaw"], color="tab:blue", lw=1)
    ax[1].set_ylabel("yaw rate (rad/s)")
    ax[2].plot(s["t"], s["est"] - s["fwd"], color="tab:gray", lw=0.8)
    ax[2].axhline(0, color="k", lw=0.5)
    ax[2].set_ylabel("speed error (m/s)")
    ax[2].set_xlabel("time in take (s)")
    if "_psd" in r:
        f, p = r["_psd"]
        ax[3].loglog(f[1:], p[1:], color="tab:blue")
        ax[3].axvline(fc, color="k", ls="--", lw=0.8)
        ax[3].text(fc, ax[3].get_ylim()[1], f" f_c = {fc} Hz", va="top")
        ax[3].set_xlabel("frequency (Hz)")
        ax[3].set_ylabel("error PSD ((m/s)²/Hz)")
    else:
        ax[3].set_axis_off()
        ax[3].text(0.5, 0.5, "no steady segment of 4 s or more: no spectrum", ha="center")
    fig.savefig(path, dpi=110)
    plt.close(fig)


def write_rerun(r, path):
    try:
        import rerun as rr
    except ImportError:
        return False
    s = r["_series"]
    rec = rr.RecordingStream(f"velocity-{r['take']}")
    rec.save(str(path))
    times = [rr.TimeColumn("take_time", duration=s["t"])]
    for entity, v in (("truth/forward", s["fwd"]), ("estimate/forward", s["est"]),
                      ("truth/yaw_rate", s["yaw"]), ("estimate/yaw_rate", s["est_yaw"]),
                      ("error/forward", s["est"] - s["fwd"]), ("estimate/ready", s["ready"].astype(float))):
        rec.send_columns(entity, indexes=times, columns=rr.Scalars.columns(scalars=v))
    return True


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--journal", required=True, help="events.db from the phone (or an exported .events.jsonl)")
    ap.add_argument("--takes", required=True, help="folder of Motive CSV exports, or one CSV")
    ap.add_argument("--out", required=True)
    ap.add_argument("--body", default="chassis", help="Motive rigid body name (default chassis)")
    ap.add_argument("--up", choices=("y", "z"), default="y", help="Motive up axis (default y)")
    ap.add_argument("--fc", type=float, default=0.4, help="loop crossover in Hz for the noise split (default 0.4)")
    ap.add_argument("--kp", type=float, default=0.3, help="linear kp for the chatter estimate (terra profile 0.3, terra_mini 1.125)")
    ap.add_argument("--sg-window", type=float, default=0.05, help="mocap differentiation window, s (default 0.05)")
    ap.add_argument("--steady-accel", type=float, default=0.15, help="m/s² below which a segment is steady")
    ap.add_argument("--heading-offset", type=float, default=None,
                    help="rigid-body yaw offset in rad; default: fitted from straight driving")
    args = ap.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    phone = load_phone(args.journal)
    tk = Path(args.takes)
    files = sorted(tk.glob("*.csv")) if tk.is_dir() else [tk]
    if not files:
        sys.exit(f"no CSV files in {tk}")
    takes = {}
    for f in files:
        m = load_motive(f, args.body, args.up)
        takes[f.stem] = truth_on_grid(m, args.sg_window)
        print(f"mocap: {f.stem}: {len(m['t'])} frames at {m['rate']:.0f} Hz")

    if args.heading_offset is None:
        offs = [o for o in (heading_offset(t) for t in takes.values()) if o is not None]
        if not offs:
            sys.exit("no straight driving found to fit the rigid-body heading; pass --heading-offset")
        args.heading_offset = float(np.angle(np.mean(np.exp(1j * np.array(offs)))))
        spread = np.degrees(np.std(np.angle(np.exp(1j * (np.array(offs) - args.heading_offset)))))
        print(f"rigid-body heading offset {math.degrees(args.heading_offset):.1f}° "
              f"(from {len(offs)} takes, spread {spread:.1f}°)")

    results = []
    for name, tr in takes.items():
        try:
            offset, sign, peak, second = align(phone, tr)
        except ValueError as e:
            print(f"{name}: skipped, {e}")
            continue
        warn = "" if peak > 0.6 and second < 0.9 * peak else "  ← weak clock match, check the sync jerks"
        print(f"\n== {name}: clock offset {offset:.3f} s, match {peak:.2f} (next {second:.2f}){warn}")
        r = analyze(name, phone, tr, offset, sign, args)
        r["clock_match"], r["clock_match_next"] = round(peak, 3), round(second, 3)
        r["verdicts"] = verdicts(r, args)
        plot(r, out / f"{name}.png", args.fc)
        try:
            if write_rerun(r, out / f"{name}.rrd"):
                r["rerun"] = f"{name}.rrd"
        except Exception as e:  # a Rerun API mismatch must not lose the metrics
            print(f"   (no Rerun recording: {e})")
        for b in r["speed_bins"]:
            print(f"   speed {b['speed']:+.2f} m/s for {b['seconds']:5.1f} s: bias {b['bias']:+.4f}  std {b['std']:.4f}")
        for k in ("scale", "latency_s", "noise_in_band", "noise_out_band", "effort_chatter", "not_ready_fraction",
                  "longest_dropout_s", "yaw_bias", "spin_spurious_forward_rms", "yaw_scale", "turning_forward_bias"):
            if k in r:
                print(f"   {k:28s} {r[k]}")
        for k, ok in r["verdicts"].items():
            print(f"   {'PASS' if ok else 'FAIL'}  {k}")
        results.append(r)

    # Scale across all takes: needs at least two distinct steady speeds.
    fit = scale_fit([x for r in results for x in r["_segmeans"]])
    pooled = None
    if fit:
        slope, icpt = fit
        pooled = {"scale": round(float(slope), 4), "intercept": round(float(icpt), 4),
                  "pass_P2": bool(0.97 <= slope <= 1.03)}
        print(f"\nall takes: scale {slope:.4f}, intercept {icpt:+.4f} m/s  "
              f"{'PASS' if pooled['pass_P2'] else 'FAIL'}  P2 scale")
    for r in results:
        r["verdicts"] = {k: bool(v) for k, v in r["verdicts"].items()}
    clean = [{k: v for k, v in r.items() if not k.startswith("_")} for r in results]
    meta = {"fc_hz": args.fc, "kp": args.kp, "heading_offset_rad": args.heading_offset,
            "phone_log_rate_hz": phone["rate"], "body": args.body, "up": args.up,
            "note": "latency is relative to the phone gyro, which sets the clock alignment"}
    (out / "summary.json").write_text(json.dumps({"settings": meta, "all_takes": pooled, "takes": clean}, indent=2))
    print(f"\nwrote {out / 'summary.json'} and a plot per take")
    if phone["rate"] < 50:
        print("note: phone logged below 50 Hz; latency and the noise spectrum are not reliable "
              "(set control_sample_seconds to 0.01)")


if __name__ == "__main__":
    main()
