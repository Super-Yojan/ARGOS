using System.Collections.Generic;
using System.Globalization;
using UnityEngine;

namespace Argos.Fleet
{
    /// <summary>
    /// One translucent quad per rover grid. Textures upload only when sequence or run id changes.
    /// </summary>
    public sealed class OccupancyLayer
    {
        readonly Transform _parent;
        readonly CampusTerrain _terrain;
        readonly Dictionary<string, Overlay> _overlays = new Dictionary<string, Overlay>();
        readonly List<string> _drop = new List<string>();

        public OccupancyLayer(Transform parent, CampusTerrain terrain)
        {
            _parent = parent;
            _terrain = terrain;
        }

        public int FreshCount { get; private set; }

        public void ApplyIndex(string indexJson)
        {
            OccupancyIndexDto index;
            try
            {
                index = OccupancyMapParser.ParseIndex(indexJson);
            }
            catch (System.Exception exception)
            {
                Debug.LogError("ARGOS occupancy index rejected: " + exception.Message);
                return;
            }

            var seen = new HashSet<string>();
            var fresh = 0;
            foreach (var map in index.maps)
            {
                if (map == null || string.IsNullOrEmpty(map.id) || !OccupancyMapParser.IsFresh(map.age))
                    continue;
                fresh++;
                seen.Add(map.id);
                if (!_overlays.TryGetValue(map.id, out var overlay))
                {
                    overlay = Create(map.id);
                    _overlays.Add(map.id, overlay);
                }

                var version = (map.runId ?? string.Empty) + ":" + map.sequence.ToString("R", CultureInfo.InvariantCulture);
                if (overlay.Version != version)
                {
                    var gridJson = ZenohBridge.PollMap((ulong)map.roverId);
                    if (!string.IsNullOrEmpty(gridJson))
                    {
                        try
                        {
                            var grid = OccupancyMapParser.ParseGrid(gridJson);
                            Upload(overlay, grid);
                            overlay.Version = version;
                        }
                        catch (System.Exception exception)
                        {
                            Debug.LogError("ARGOS occupancy grid rejected: " + exception.Message);
                        }
                    }
                }

                Place(overlay, map);
            }

            _drop.Clear();
            foreach (var pair in _overlays)
            {
                if (!seen.Contains(pair.Key))
                    _drop.Add(pair.Key);
            }

            for (var i = 0; i < _drop.Count; i++)
            {
                var id = _drop[i];
                if (_overlays.TryGetValue(id, out var overlay) && overlay.Root != null)
                    Object.Destroy(overlay.Root);
                _overlays.Remove(id);
            }

            FreshCount = fresh;
        }

        public void ApplySelection(string selectedId)
        {
            var dimOthers = !string.IsNullOrEmpty(selectedId);
            foreach (var pair in _overlays)
            {
                if (pair.Value.Material == null)
                    continue;
                var alpha = dimOthers && pair.Key != selectedId ? 0.22f : 1f;
                pair.Value.Material.SetColor("_Color", new Color(1f, 1f, 1f, alpha));
            }
        }

        static Overlay Create(string id)
        {
            var root = new GameObject("occupancy-" + id);
            var mesh = new Mesh { name = "occupancy-" + id };
            mesh.vertices = new[]
            {
                new Vector3(0f, 0f, 0f),
                new Vector3(1f, 0f, 0f),
                new Vector3(1f, 0f, 1f),
                new Vector3(0f, 0f, 1f),
            };
            mesh.uv = new[]
            {
                new Vector2(0f, 0f),
                new Vector2(1f, 0f),
                new Vector2(1f, 1f),
                new Vector2(0f, 1f),
            };
            mesh.triangles = new[] { 0, 2, 1, 0, 3, 2 };
            mesh.RecalculateBounds();

            var filter = root.AddComponent<MeshFilter>();
            filter.sharedMesh = mesh;
            var renderer = root.AddComponent<MeshRenderer>();
            var shader = Shader.Find("Argos/OccupancyOverlay");
            if (shader == null)
                shader = Shader.Find("Sprites/Default");
            if (shader == null)
                shader = Shader.Find("Unlit/Transparent");
            var material = new Material(shader);
            material.SetColor("_Color", Color.white);
            renderer.sharedMaterial = material;
            renderer.shadowCastingMode = UnityEngine.Rendering.ShadowCastingMode.Off;
            renderer.receiveShadows = false;
            return new Overlay { Root = root, Material = material, Mesh = mesh };
        }

        void Place(Overlay overlay, OccupancyMapDto map)
        {
            overlay.Root.transform.SetParent(_parent, false);
            var sizeX = map.width * map.resolution;
            var sizeZ = map.height * map.resolution;
            var centerX = map.originX + sizeX * 0.5f;
            var centerZ = map.originY + sizeZ * 0.5f;
            var height = _terrain != null ? _terrain.Sample(centerX, centerZ) : 0f;
            overlay.Root.transform.position = new Vector3(map.originX, height + 0.18f, map.originY);
            overlay.Root.transform.localScale = new Vector3(sizeX, 1f, sizeZ);
        }

        static void Upload(Overlay overlay, OccupancyMapDto grid)
        {
            if (overlay.Texture == null || overlay.Texture.width != grid.width || overlay.Texture.height != grid.height)
            {
                if (overlay.Texture != null)
                    Object.Destroy(overlay.Texture);
                overlay.Texture = new Texture2D(grid.width, grid.height, TextureFormat.RGBA32, false)
                {
                    wrapMode = TextureWrapMode.Clamp,
                    filterMode = FilterMode.Point,
                };
                overlay.Material.mainTexture = overlay.Texture;
            }

            var pixels = new Color32[grid.occupancy.Length];
            for (var i = 0; i < pixels.Length; i++)
                pixels[i] = CellColor(grid.occupancy[i]);
            overlay.Texture.SetPixels32(pixels);
            overlay.Texture.Apply(false);
        }

        static Color32 CellColor(int cell)
        {
            if (cell < 0)
                return new Color32(0, 0, 0, 0);
            if (cell == 0)
                return new Color32(176, 184, 192, 28);
            var alpha = (byte)Mathf.Clamp(70 + cell, 70, 170);
            return new Color32(196, 200, 206, alpha);
        }

        sealed class Overlay
        {
            public GameObject Root;
            public Material Material;
            public Mesh Mesh;
            public Texture2D Texture;
            public string Version = string.Empty;
        }
    }
}
