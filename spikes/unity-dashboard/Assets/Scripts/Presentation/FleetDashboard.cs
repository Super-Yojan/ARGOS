using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using UnityEngine;

namespace Argos.Fleet
{
    /// <summary>
    /// Rough level-1 / level-2 fleet view. Mock JSON only. No Zenoh.
    /// </summary>
    public sealed class FleetDashboard : MonoBehaviour
    {
        const string TelemetryFileName = "fleet-telemetry.json";
        const string AutoSelectId = "terra-07";

        readonly List<RoverVisual> _rovers = new List<RoverVisual>();
        string _selectedId = string.Empty;
        float _elapsed;
        bool _overviewShot;
        bool _selectedShot;
        bool _autoSelectArmed = true;
        string _captureDir = string.Empty;
        bool _quitAfterCapture;

        public string SelectedId => _selectedId;
        public int RoverCount => _rovers.Count;

        void Awake()
        {
            _captureDir = SpikeArgs.Get("-argosCaptureDir", string.Empty);
            _quitAfterCapture = SpikeArgs.Has("-argosQuit");
            _autoSelectArmed = !SpikeArgs.Has("-argosNoDemo");
            BuildGround();
            LoadAndSpawn();
            WritePresentationDump();
        }

        void Update()
        {
            _elapsed += Time.unscaledDeltaTime;
            if (_autoSelectArmed && _elapsed >= 6.5f && string.IsNullOrEmpty(_selectedId))
                Select(AutoSelectId);

            if (Input.GetKeyDown(KeyCode.Escape))
                Select(string.Empty);

            PulseColors();
            MaybeCapture();
        }

        public void Select(string roverId)
        {
            _selectedId = roverId ?? string.Empty;
            ApplyColors(pulse: 1f);
            Debug.Log("ARGOS select: '" + _selectedId + "'");
        }

        void LoadAndSpawn()
        {
            var path = Path.Combine(Application.streamingAssetsPath, TelemetryFileName);
            if (!File.Exists(path))
            {
                Debug.LogError("ARGOS telemetry file missing: " + path);
                return;
            }

            var snapshot = TelemetryParser.Parse(File.ReadAllText(path));
            var now = snapshot.snapshotUnixSeconds;
            var stale = 0;
            var attention = 0;
            var critical = 0;

            foreach (var rover in snapshot.rovers)
            {
                var view = TelemetryParser.Evaluate(rover, now);
                if (view.IsStale)
                    stale++;
                if (view.Status == RoverStatus.Attention)
                    attention++;
                if (view.Status == RoverStatus.Critical)
                    critical++;
                _rovers.Add(Spawn(rover, view));
            }

            Debug.Log(string.Format(
                CultureInfo.InvariantCulture,
                "ARGOS fleet: count={0} stale={1} attention={2} critical={3} snapshot={4}",
                _rovers.Count,
                stale,
                attention,
                critical,
                now));
        }

        RoverVisual Spawn(RoverDto rover, RoverView view)
        {
            var root = new GameObject(string.IsNullOrEmpty(view.Id) ? "rover" : view.Id);
            root.transform.SetParent(transform, false);
            root.transform.position = view.Position;
            root.transform.rotation = Quaternion.Euler(0f, view.HeadingDegrees, 0f);

            var body = GameObject.CreatePrimitive(PrimitiveType.Cube);
            body.name = "body";
            body.transform.SetParent(root.transform, false);
            body.transform.localScale = new Vector3(0.85f, 0.28f, 1.25f);
            body.transform.localPosition = new Vector3(0f, 0.2f, 0f);

            var nose = GameObject.CreatePrimitive(PrimitiveType.Cube);
            nose.name = "nose";
            nose.transform.SetParent(root.transform, false);
            nose.transform.localScale = new Vector3(0.28f, 0.16f, 0.28f);
            nose.transform.localPosition = new Vector3(0f, 0.34f, 0.62f);

            var ring = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            ring.name = "selection-ring";
            ring.transform.SetParent(root.transform, false);
            ring.transform.localScale = new Vector3(1.7f, 0.015f, 1.7f);
            ring.transform.localPosition = new Vector3(0f, 0.02f, 0f);
            ring.SetActive(false);

            var marker = root.AddComponent<RoverPick>();
            marker.Dashboard = this;
            marker.RoverId = view.Id;

            var visual = new RoverVisual
            {
                Id = view.Id,
                Status = view.Status,
                Stale = view.IsStale,
                Root = root,
                Body = body.GetComponent<Renderer>(),
                Nose = nose.GetComponent<Renderer>(),
                Ring = ring,
                RingRenderer = ring.GetComponent<Renderer>(),
                Completed = BuildLine(root.transform, "completed", ToWorld(rover.completedPath), dashed: false),
                Planned = BuildDashed(root.transform, "planned", ToWorld(rover.plannedPath)),
            };

            visual.Body.sharedMaterial = CreateMaterial();
            visual.Nose.sharedMaterial = CreateMaterial();
            visual.RingRenderer.sharedMaterial = CreateMaterial();
            return visual;
        }

        void BuildGround()
        {
            var ground = GameObject.CreatePrimitive(PrimitiveType.Plane);
            ground.name = "ground";
            ground.transform.SetParent(transform, false);
            ground.transform.position = new Vector3(0f, -0.02f, 0f);
            ground.transform.localScale = new Vector3(3.2f, 1f, 2.6f);
            var renderer = ground.GetComponent<Renderer>();
            renderer.sharedMaterial = CreateOpaqueMaterial();
            renderer.sharedMaterial.SetColor("_Color", new Color(0.035f, 0.045f, 0.06f, 1f));
            var collider = ground.GetComponent<Collider>();
            if (collider != null)
                collider.enabled = false;
        }

        void ApplyColors(float pulse)
        {
            var somethingSelected = !string.IsNullOrEmpty(_selectedId);
            foreach (var rover in _rovers)
            {
                var selected = somethingSelected && rover.Id == _selectedId;
                var dimmed = somethingSelected && !selected;
                var color = StatusLanguage.Resolve(rover.Status, rover.Stale, dimmed);
                if (!rover.Stale && !dimmed && (rover.Status == RoverStatus.Attention || rover.Status == RoverStatus.Critical))
                {
                    var wave = 0.72f + 0.28f * pulse;
                    color.r *= wave;
                    color.g *= wave;
                    color.b *= wave;
                }

                SetColor(rover.Body, color);
                SetColor(rover.Nose, Color.Lerp(color, Color.white, rover.Stale ? 0f : 0.25f));
                rover.Ring.SetActive(selected);
                if (selected)
                    SetColor(rover.RingRenderer, StatusLanguage.Cyan);

                TintPaths(rover.Completed, color);
                var planned = color;
                planned.a *= 0.85f;
                TintPaths(rover.Planned, planned);
            }
        }

        void PulseColors()
        {
            var pulse = 0.5f + 0.5f * Mathf.Sin(Time.unscaledTime * 3.2f);
            ApplyColors(pulse);
        }

        static void TintPaths(IList<LineRenderer> lines, Color color)
        {
            if (lines == null)
                return;
            for (var i = 0; i < lines.Count; i++)
            {
                var line = lines[i];
                if (line == null)
                    continue;
                line.startColor = color;
                line.endColor = color;
                if (line.material != null)
                    line.material.SetColor("_Color", color);
            }
        }

        static void SetColor(Renderer renderer, Color color)
        {
            if (renderer == null || renderer.sharedMaterial == null)
                return;
            renderer.sharedMaterial.SetColor("_Color", color);
        }

        List<LineRenderer> BuildLine(Transform parent, string name, Vector3[] points, bool dashed)
        {
            var lines = new List<LineRenderer>();
            if (points == null || points.Length < 2)
                return lines;

            if (!dashed)
            {
                lines.Add(MakeLine(parent, name, points, 0.07f));
                return lines;
            }

            return BuildDashed(parent, name, points);
        }

        List<LineRenderer> BuildDashed(Transform parent, string name, Vector3[] points)
        {
            var lines = new List<LineRenderer>();
            var dashes = PathDasher.BuildDashes(points, 0.45f, 0.28f);
            for (var i = 0; i < dashes.Length; i++)
                lines.Add(MakeLine(parent, name + "-" + i.ToString(CultureInfo.InvariantCulture), dashes[i], 0.045f));
            return lines;
        }

        static LineRenderer MakeLine(Transform parent, string name, Vector3[] points, float width)
        {
            var go = new GameObject(name);
            go.transform.SetParent(parent, false);
            var line = go.AddComponent<LineRenderer>();
            line.useWorldSpace = true;
            line.positionCount = points.Length;
            line.SetPositions(points);
            line.startWidth = width;
            line.endWidth = width;
            line.material = CreateMaterial();
            line.shadowCastingMode = UnityEngine.Rendering.ShadowCastingMode.Off;
            line.receiveShadows = false;
            return line;
        }

        static Vector3[] ToWorld(Vec3Dto[] points)
        {
            if (points == null || points.Length == 0)
                return new Vector3[0];

            var world = new Vector3[points.Length];
            for (var i = 0; i < points.Length; i++)
            {
                var point = points[i] != null ? points[i].ToVector3() : Vector3.zero;
                point.y += 0.06f;
                world[i] = point;
            }

            return world;
        }

        static Material CreateMaterial()
        {
            var shader = Shader.Find("Argos/FleetUnlit");
            if (shader == null)
                shader = Shader.Find("Sprites/Default");
            if (shader == null)
                shader = Shader.Find("Unlit/Color");
            if (shader == null)
                shader = Shader.Find("Standard");
            return new Material(shader);
        }

        static Material CreateOpaqueMaterial()
        {
            var shader = Shader.Find("Argos/FleetOpaque");
            if (shader == null)
                shader = Shader.Find("Unlit/Color");
            if (shader == null)
                return CreateMaterial();
            return new Material(shader);
        }

        void MaybeCapture()
        {
            if (!string.IsNullOrEmpty(_captureDir))
            {
                if (!_overviewShot && _elapsed >= 4.5f)
                {
                    _overviewShot = true;
                    Capture("overview");
                }

                if (!_selectedShot && _elapsed >= 9f)
                {
                    _selectedShot = true;
                    Capture("selected");
                }
            }

            if (_quitAfterCapture && _elapsed >= 12f)
            {
                Debug.Log("ARGOS capture window finished, quitting.");
                Application.Quit(0);
            }
        }

        void Capture(string name)
        {
            Directory.CreateDirectory(_captureDir);
            var path = Path.Combine(_captureDir, name + ".png");
            ScreenCapture.CaptureScreenshot(path);
            Debug.Log("ARGOS screenshot requested: " + path);
        }

        void WritePresentationDump()
        {
            if (string.IsNullOrEmpty(_captureDir))
                return;

            Directory.CreateDirectory(_captureDir);
            var builder = new StringBuilder();
            builder.AppendLine("id status stale");
            foreach (var rover in _rovers)
            {
                var color = StatusLanguage.Resolve(rover.Status, rover.Stale, false);
                builder.AppendFormat(
                    CultureInfo.InvariantCulture,
                    "{0} {1} stale={2} rgba={3:0.00},{4:0.00},{5:0.00},{6:0.00}",
                    rover.Id,
                    rover.Status,
                    rover.Stale,
                    color.r,
                    color.g,
                    color.b,
                    color.a);
                builder.AppendLine();
            }

            File.WriteAllText(Path.Combine(_captureDir, "presentation.txt"), builder.ToString());
        }

        sealed class RoverVisual
        {
            public string Id;
            public RoverStatus Status;
            public bool Stale;
            public GameObject Root;
            public Renderer Body;
            public Renderer Nose;
            public GameObject Ring;
            public Renderer RingRenderer;
            public List<LineRenderer> Completed;
            public List<LineRenderer> Planned;
        }
    }

    public sealed class RoverPick : MonoBehaviour
    {
        public FleetDashboard Dashboard;
        public string RoverId;

        void OnMouseDown()
        {
            if (Dashboard != null)
                Dashboard.Select(RoverId);
        }
    }

    static class SpikeArgs
    {
        public static bool Has(string flag)
        {
            var args = Environment.GetCommandLineArgs();
            for (var i = 0; i < args.Length; i++)
            {
                if (args[i] == flag)
                    return true;
            }

            return false;
        }

        public static string Get(string key, string fallback)
        {
            var args = Environment.GetCommandLineArgs();
            for (var i = 0; i < args.Length - 1; i++)
            {
                if (args[i] == key)
                    return args[i + 1];
            }

            return fallback;
        }
    }
}
