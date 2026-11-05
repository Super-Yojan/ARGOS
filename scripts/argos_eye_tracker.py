#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9,<3.13"
# dependencies = ["mediapipe==0.10.21", "opencv-python>=4.8", "numpy>=1.24"]
# ///
"""Webcam eye tracker for ARGOS: gaze point on screen and blinks.

Run it next to ARGOS on the same computer, with ARGOS on the main display:

    uv run scripts/argos_eye_tracker.py              # calibrate, then track
    uv run scripts/argos_eye_tracker.py --simulate   # no camera: fake gaze for testing

It sends one JSON object per UDP datagram to 127.0.0.1:47870 (ARGOS_GAZE_PORT):

    {"type": "gaze", "utc": ..., "x": 812.4, "y": 433.0, "confidence": 0.9}   screen points
    {"type": "no_face", "utc": ...}
    {"type": "blink", "utc": ..., "duration_ms": 140}
    {"type": "status", "utc": ..., "tracker": "webcam-mediapipe", "calibrated": true, "error_px": 61, ...}

ARGOS maps each point to the rover card, map marker or panel under it and
writes gaze_dwell, gaze_sample and blink records to the session log.

How it works: MediaPipe Face Mesh finds the irises and eyelids in each frame.
Gaze features are where each iris sits inside its eye opening plus a rough head
pose; a 2nd-order ridge regression fitted during a 13-point calibration maps
them to the screen. Blinks are drops of the eye aspect ratio (EAR) below 70 %
of the operator's open-eye EAR. Expect a few degrees of error (roughly 50 to
100 points at 60 cm): good enough to tell which rover card or map area someone
looks at, not which word they read. Keep the head roughly where it was during
calibration, and recalibrate (--recalibrate) after moving the laptop or chair.
"""
import argparse
import json
import math
import os
import random
import socket
import sys
import time
from pathlib import Path

CALIBRATION_FILE = Path.home() / ".argos" / "eye-calibration.json"
TRACKER = "webcam-mediapipe"

# Face Mesh landmark indices (refine_landmarks=True adds the irises, 468-477).
RIGHT_EYE = dict(outer=33, inner=133, top=159, bottom=145, ear=(33, 160, 158, 133, 153, 144), iris=468)
LEFT_EYE = dict(outer=263, inner=362, top=386, bottom=374, ear=(362, 385, 387, 263, 373, 380), iris=473)
NOSE_TIP, CHIN, FOREHEAD = 1, 152, 10

BLINK_RATIO = 0.70  # EAR below this share of the open-eye EAR is a closed eye
BLINK_MIN_MS = 50
BLINK_MAX_MS = 700  # longer closures are sent too, but flagged by duration
SMOOTHING = 0.35  # exponential smoothing of the gaze point (1 = none)


class Sender:
    def __init__(self, host, port, csv_path=None):
        self.address = (host, port)
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.csv = open(csv_path, "a", buffering=1) if csv_path else None

    def send(self, message):
        message.setdefault("utc", time.time())
        line = json.dumps(message)
        try:
            self.socket.sendto(line.encode(), self.address)
        except OSError:
            pass  # ARGOS not running yet; keep tracking
        if self.csv:
            self.csv.write(line + "\n")


# ---------------------------------------------------------------- features

def _point(landmarks, index, width, height):
    p = landmarks[index]
    return p.x * width, p.y * height


def eye_aspect_ratio(landmarks, eye, width, height):
    p = [_point(landmarks, i, width, height) for i in eye["ear"]]
    d = lambda a, b: math.dist(a, b)
    horizontal = d(p[0], p[3])
    return 0.0 if horizontal == 0 else (d(p[1], p[5]) + d(p[2], p[4])) / (2.0 * horizontal)


def gaze_features(landmarks, width, height):
    """Iris position inside each eye opening, plus head position and turn."""
    features = []
    for eye in (RIGHT_EYE, LEFT_EYE):
        ix, iy = _point(landmarks, eye["iris"], width, height)
        ox, oy = _point(landmarks, eye["outer"], width, height)
        nx, ny = _point(landmarks, eye["inner"], width, height)
        tx, ty = _point(landmarks, eye["top"], width, height)
        bx, by = _point(landmarks, eye["bottom"], width, height)
        span = math.dist((ox, oy), (nx, ny)) or 1.0
        # Project the iris onto the corner-to-corner axis (horizontal ratio)
        ux, uy = (nx - ox) / span, (ny - oy) / span
        horizontal = ((ix - ox) * ux + (iy - oy) * uy) / span
        opening = math.dist((tx, ty), (bx, by)) or 1.0
        vertical = ((ix - tx) * -uy + (iy - ty) * ux) / opening
        features += [horizontal, vertical]
    nose = _point(landmarks, NOSE_TIP, width, height)
    r_outer = _point(landmarks, RIGHT_EYE["outer"], width, height)
    l_outer = _point(landmarks, LEFT_EYE["outer"], width, height)
    mid = ((r_outer[0] + l_outer[0]) / 2, (r_outer[1] + l_outer[1]) / 2)
    face = math.dist(r_outer, l_outer) or 1.0
    features += [
        (nose[0] - mid[0]) / face,  # head yaw
        (nose[1] - mid[1]) / face,  # head pitch
        mid[0] / width,  # head position in the frame
        mid[1] / height,
    ]
    return features


def expand(features):
    """Second-order terms for the regression."""
    out = [1.0] + list(features)
    n = len(features)
    for i in range(n):
        for j in range(i, n):
            out.append(features[i] * features[j])
    return out


class GazeModel:
    def __init__(self, weights=None, error_px=None, open_ear=None):
        self.weights = weights
        self.error_px = error_px
        self.open_ear = open_ear

    def fit(self, samples, ridge=1e-3):
        import numpy as np

        x = np.array([expand(f) for f, _ in samples])
        y = np.array([t for _, t in samples])
        scale = x.std(axis=0)
        scale[scale == 0] = 1.0
        xs = x / scale
        a = xs.T @ xs + ridge * len(xs) * np.eye(xs.shape[1])
        w = np.linalg.solve(a, xs.T @ y) / scale[:, None]
        self.weights = w.tolist()

    def predict(self, features):
        row = expand(features)
        return tuple(sum(r * w[k] for r, w in zip(row, self.weights)) for k in range(2))

    def save(self, path, screen):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(dict(weights=self.weights, error_px=self.error_px,
                                        open_ear=self.open_ear, screen=screen, saved=time.time())))

    @classmethod
    def load(cls, path, screen):
        data = json.loads(path.read_text())
        if data.get("screen") != list(screen):
            raise ValueError("calibration was made on a different screen size")
        return cls(data["weights"], data.get("error_px"), data.get("open_ear"))


# ---------------------------------------------------------------- camera

class FaceTracker:
    def __init__(self, camera):
        import cv2
        import mediapipe as mp

        self.cv2 = cv2
        self.capture = cv2.VideoCapture(camera)
        if not self.capture.isOpened():
            sys.exit(f"Could not open camera {camera}. On macOS, allow camera access for your terminal.")
        self.capture.set(cv2.CAP_PROP_FRAME_WIDTH, 1280)
        self.capture.set(cv2.CAP_PROP_FRAME_HEIGHT, 720)
        self.mesh = mp.solutions.face_mesh.FaceMesh(
            max_num_faces=1, refine_landmarks=True,
            min_detection_confidence=0.5, min_tracking_confidence=0.5)

    def read(self):
        """(features, ear, frame) or (None, None, frame) when no face is seen."""
        ok, frame = self.capture.read()
        if not ok:
            return None, None, None
        frame = self.cv2.flip(frame, 1)
        height, width = frame.shape[:2]
        result = self.mesh.process(self.cv2.cvtColor(frame, self.cv2.COLOR_BGR2RGB))
        if not result.multi_face_landmarks:
            return None, None, frame
        lm = result.multi_face_landmarks[0].landmark
        ear = (eye_aspect_ratio(lm, RIGHT_EYE, width, height) + eye_aspect_ratio(lm, LEFT_EYE, width, height)) / 2
        return gaze_features(lm, width, height), ear, frame


class BlinkDetector:
    def __init__(self, open_ear):
        self.open_ear = open_ear
        self.closed_since = None
        self.history = []

    def update(self, ear, now):
        """Returns a blink duration in ms when the eyes reopen, else None.
        Also returns whether the eyes are closed right now."""
        if ear is None:
            return None, False
        if self.open_ear is None or ear > self.open_ear * 0.9:
            self.history = (self.history + [ear])[-300:]
            self.open_ear = sorted(self.history)[len(self.history) // 2]
        closed = ear < self.open_ear * BLINK_RATIO
        if closed and self.closed_since is None:
            self.closed_since = now
        if not closed and self.closed_since is not None:
            ms = (now - self.closed_since) * 1000
            self.closed_since = None
            if ms >= BLINK_MIN_MS:
                return ms, False
        return None, closed


# ---------------------------------------------------------------- calibration

def calibration_points(width, height):
    xs, ys = (0.08, 0.5, 0.92), (0.1, 0.5, 0.9)
    points = [(x * width, y * height) for y in ys for x in xs]
    points += [(0.3 * width, 0.3 * height), (0.7 * width, 0.3 * height),
               (0.3 * width, 0.7 * height), (0.7 * width, 0.7 * height)]
    random.shuffle(points)
    return points


def calibrate(tracker, sender):
    """Full-screen dots: look at each one until it moves. Returns (model, screen)."""
    import tkinter as tk

    root = tk.Tk()
    root.attributes("-fullscreen", True)
    root.configure(background="#20262c", cursor="none")
    root.update()
    width, height = root.winfo_screenwidth(), root.winfo_screenheight()
    canvas = tk.Canvas(root, width=width, height=height, background="#20262c", highlightthickness=0)
    canvas.pack()
    targets = calibration_points(width, height)
    checks = [(0.25 * width, 0.5 * height), (0.75 * width, 0.5 * height),
              (0.5 * width, 0.2 * height), (0.5 * width, 0.8 * height)]
    samples, check_samples, ears = [], [], []
    state = dict(index=0, phase="intro", started=time.time(), check=False)
    settle, collect = 0.8, 1.2

    def draw(x, y, text=None):
        canvas.delete("all")
        canvas.create_oval(x - 14, y - 14, x + 14, y + 14, fill="#e8a443", outline="")
        canvas.create_oval(x - 3, y - 3, x + 3, y + 3, fill="#20262c", outline="")
        if text:
            canvas.create_text(width / 2, height - 60, text=text, fill="#cfd8de", font=("Helvetica", 20))

    def step():
        now = time.time()
        features, ear, _ = tracker.read()
        points = checks if state["check"] else targets
        if state["phase"] == "intro":
            canvas.delete("all")
            canvas.create_text(width / 2, height / 2, fill="#e4eaef", font=("Helvetica", 26), text=(
                "ARGOS eye tracker calibration\n\nKeep your head still and look at each orange dot.\n"
                "Starting in a moment. Esc cancels."), justify="center")
            if now - state["started"] > 3:
                state.update(phase="dots", started=now)
        else:
            if state["index"] >= len(points):
                if state["check"]:
                    root.quit()
                    return
                state.update(check=True, index=0, started=now)
                points = checks
            x, y = points[state["index"]]
            label = "Checking accuracy" if state["check"] else f"Point {state['index'] + 1} of {len(points)}"
            draw(x, y, label)
            age = now - state["started"]
            if settle <= age <= settle + collect and features is not None:
                (check_samples if state["check"] else samples).append((features, (x, y)))
                if ear is not None:
                    ears.append(ear)
            if age > settle + collect:
                state.update(index=state["index"] + 1, started=now)
        root.after(5, step)

    root.bind("<Escape>", lambda _: sys.exit("Calibration cancelled"))
    root.after(10, step)
    root.mainloop()
    root.destroy()
    if len(samples) < 60:
        sys.exit("Calibration failed: the camera did not see your eyes. Check lighting and camera position.")
    model = GazeModel()
    model.fit(samples)
    errors = [math.dist(model.predict(f), t) for f, t in check_samples]
    model.error_px = round(sorted(errors)[len(errors) // 2], 1) if errors else None
    model.open_ear = sorted(ears)[len(ears) // 2] if ears else None
    print(f"Calibrated: median error {model.error_px} points on a {width}x{height} screen")
    sender.send(dict(type="status", tracker=TRACKER, event="calibrated", calibrated=True,
                     error_px=model.error_px, screen=[width, height], points=len(targets)))
    return model, (width, height)


def screen_size():
    import tkinter as tk

    root = tk.Tk()
    root.withdraw()
    size = (root.winfo_screenwidth(), root.winfo_screenheight())
    root.destroy()
    return size


# ---------------------------------------------------------------- loops

def track(args, sender):
    tracker = FaceTracker(args.camera)
    screen = screen_size()
    model = None
    if not args.recalibrate and CALIBRATION_FILE.exists():
        try:
            model = GazeModel.load(CALIBRATION_FILE, screen)
            print(f"Using saved calibration ({model.error_px} points error). --recalibrate to redo it.")
        except (ValueError, KeyError, json.JSONDecodeError) as error:
            print(f"Saved calibration not usable: {error}")
    if model is None:
        model, screen = calibrate(tracker, sender)
        model.save(CALIBRATION_FILE, list(screen))
    blink = BlinkDetector(model.open_ear)
    smooth, last_status, last_no_face, blinks, frames = None, 0.0, 0.0, [], 0
    started = time.time()
    print("Tracking. Ctrl-C to stop.")
    while True:
        features, ear, frame = tracker.read()
        now = time.time()
        frames += 1
        if features is None:
            smooth = None
            if now - last_no_face > 0.5:
                sender.send(dict(type="no_face", utc=now))
                last_no_face = now
        else:
            duration, closed = blink.update(ear, now)
            if duration is not None:
                sender.send(dict(type="blink", utc=now, duration_ms=round(duration),
                                 long=duration > BLINK_MAX_MS))
                blinks.append(now)
            if not closed:
                x, y = model.predict(features)
                x = min(max(x, -0.1 * screen[0]), 1.1 * screen[0])
                y = min(max(y, -0.1 * screen[1]), 1.1 * screen[1])
                smooth = (x, y) if smooth is None else (
                    smooth[0] + SMOOTHING * (x - smooth[0]), smooth[1] + SMOOTHING * (y - smooth[1]))
                sender.send(dict(type="gaze", utc=now, x=round(smooth[0], 1), y=round(smooth[1], 1),
                                 confidence=1.0))
        if now - last_status > 5:
            blinks = [t for t in blinks if now - t < 60]
            sender.send(dict(type="status", utc=now, tracker=TRACKER, calibrated=True,
                             error_px=model.error_px, screen=list(screen),
                             fps=round(frames / max(now - started, 1e-6), 1),
                             blinks_last_min=len(blinks), open_ear=round(blink.open_ear or 0, 3)))
            last_status = now
        if args.preview and frame is not None:
            tracker.cv2.imshow("ARGOS eye tracker (q quits)", frame)
            if tracker.cv2.waitKey(1) & 0xFF == ord("q"):
                break


def simulate(args, sender):
    """Fake operator: looks around the screen, dwells, blinks ~15 times a minute."""
    width, height = args.screen
    print(f"Simulating gaze on a {width}x{height} screen to port {args.port}. Ctrl-C to stop.")
    sender.send(dict(type="status", tracker="simulated", calibrated=True, error_px=0, screen=[width, height]))
    target, until, next_blink = (width / 2, height / 2), 0.0, time.time() + 4
    while True:
        now = time.time()
        if now > until:
            target = (random.uniform(0, width), random.uniform(0, height))
            until = now + random.uniform(0.3, 2.5)
        if now > next_blink:
            sender.send(dict(type="blink", utc=now, duration_ms=random.randint(90, 250)))
            next_blink = now + random.expovariate(1 / 4.0)
        sender.send(dict(type="gaze", utc=now, x=round(target[0] + random.gauss(0, 15), 1),
                         y=round(target[1] + random.gauss(0, 15), 1), confidence=1.0))
        time.sleep(1 / 30)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--camera", type=int, default=0)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=int(os.environ.get("ARGOS_GAZE_PORT", 47870)))
    parser.add_argument("--recalibrate", action="store_true", help="ignore the saved calibration")
    parser.add_argument("--preview", action="store_true", help="show the camera image")
    parser.add_argument("--csv", help="also append every message to this file (JSON lines)")
    parser.add_argument("--simulate", action="store_true", help="no camera: send fake gaze and blinks")
    parser.add_argument("--screen", type=int, nargs=2, default=(1512, 982), metavar=("W", "H"),
                        help="screen size in points for --simulate")
    args = parser.parse_args()
    sender = Sender(args.host, args.port, args.csv)
    try:
        (simulate if args.simulate else track)(args, sender)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
