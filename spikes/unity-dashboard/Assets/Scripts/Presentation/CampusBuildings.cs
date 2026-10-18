using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using UnityEngine;
using UnityEngine.Rendering;

namespace Argos.Fleet
{
    /// <summary>
    /// OSM height, then building:levels times 3.5 m, otherwise three of those levels.
    /// </summary>
    public static class BuildingHeight
    {
        public const float MetersPerLevel = 3.5f;
        public const float DefaultMeters = 10.5f;

        public static float Resolve(string height, string levels, out string source)
        {
            if (TryParseHeight(height, out var meters))
            {
                source = "height";
                return meters;
            }

            if (float.TryParse(levels, NumberStyles.Float, CultureInfo.InvariantCulture, out var count) && count > 0f)
            {
                source = "levels";
                return count * MetersPerLevel;
            }

            source = "default";
            return DefaultMeters;
        }

        static bool TryParseHeight(string height, out float meters)
        {
            meters = 0f;
            if (string.IsNullOrWhiteSpace(height))
                return false;

            var text = height.Trim().ToLowerInvariant();
            var feet = text.EndsWith("ft", StringComparison.Ordinal);
            if (feet)
                text = text.Substring(0, text.Length - 2).Trim();
            else if (text.EndsWith("m", StringComparison.Ordinal))
                text = text.Substring(0, text.Length - 1).Trim();

            if (!float.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var value) || value <= 0f)
                return false;

            meters = feet ? value * 0.3048f : value;
            return true;
        }
    }

    /// <summary>
    /// Ear-clip a simple footprint. The list is reordered counter-clockwise in x/z.
    /// </summary>
    public static class PolygonTriangulator
    {
        public static int[] Triangulate(List<Vector2> polygon)
        {
            if (polygon == null || polygon.Count < 3)
                return Array.Empty<int>();

            if (SignedArea(polygon) < 0f)
                polygon.Reverse();

            var index = new List<int>(polygon.Count);
            for (var i = 0; i < polygon.Count; i++)
                index.Add(i);

            var triangles = new List<int>((polygon.Count - 2) * 3);
            var guard = 0;
            while (index.Count > 3 && guard < 10000)
            {
                guard++;
                var count = index.Count;
                var clipped = false;
                for (var i = 0; i < count; i++)
                {
                    var i0 = index[(i - 1 + count) % count];
                    var i1 = index[i];
                    var i2 = index[(i + 1) % count];
                    var a = polygon[i0];
                    var b = polygon[i1];
                    var c = polygon[i2];
                    if (Cross(a, b, c) <= 1e-5f)
                        continue;

                    var ear = true;
                    for (var k = 0; k < index.Count; k++)
                    {
                        var j = index[k];
                        if (j == i0 || j == i1 || j == i2)
                            continue;
                        var point = polygon[j];
                        if (Cross(a, b, point) >= -1e-5f
                            && Cross(b, c, point) >= -1e-5f
                            && Cross(c, a, point) >= -1e-5f)
                        {
                            ear = false;
                            break;
                        }
                    }

                    if (!ear)
                        continue;

                    triangles.Add(i0);
                    triangles.Add(i1);
                    triangles.Add(i2);
                    index.RemoveAt(i);
                    clipped = true;
                    break;
                }

                if (!clipped)
                    return Fan(polygon.Count);
            }

            if (index.Count == 3)
            {
                triangles.Add(index[0]);
                triangles.Add(index[1]);
                triangles.Add(index[2]);
            }

            return triangles.ToArray();
        }

        static int[] Fan(int count)
        {
            var triangles = new int[(count - 2) * 3];
            var cursor = 0;
            for (var i = 1; i < count - 1; i++)
            {
                triangles[cursor++] = 0;
                triangles[cursor++] = i;
                triangles[cursor++] = i + 1;
            }

            return triangles;
        }

        static float SignedArea(List<Vector2> polygon)
        {
            var area = 0f;
            for (var i = 0; i < polygon.Count; i++)
            {
                var a = polygon[i];
                var b = polygon[(i + 1) % polygon.Count];
                area += a.x * b.y - b.x * a.y;
            }

            return area;
        }

        static float Cross(Vector2 origin, Vector2 a, Vector2 b)
        {
            return (a.x - origin.x) * (b.y - origin.y) - (a.y - origin.y) * (b.x - origin.x);
        }
    }

    [Serializable]
    public sealed class CampusBuildingFile
    {
        public string attribution;
        public double anchorLatitude;
        public double anchorLongitude;
        public float defaultHeightMeters;
        public float levelHeightMeters;
        public int withHeight;
        public int withLevels;
        public int withDefault;
        public CampusBuildingDto[] buildings;
    }

    [Serializable]
    public sealed class CampusBuildingDto
    {
        public string osmId;
        public string name;
        public float heightMeters;
        public string heightSource;
        public GeoPointDto[] outline;
    }

    [Serializable]
    public sealed class GeoPointDto
    {
        public double lat;
        public double lon;
    }

    /// <summary>
    /// Quiet blocks from the bundled OpenStreetMap extract. They sit on the terrain
    /// and draw before occupancy grids. Rovers use a later queue and ignore depth.
    /// </summary>
    public static class CampusBuildings
    {
        public static int TryCreate(Transform parent, CampusTerrain terrain)
        {
            var path = Path.Combine(Application.streamingAssetsPath, "buildings", "gmu-fairfax.json");
            if (!File.Exists(path))
            {
                Debug.LogError("ARGOS buildings missing: " + path);
                LogCounts(0, 0, 0, 0);
                return 0;
            }

            CampusBuildingFile file;
            try
            {
                file = JsonUtility.FromJson<CampusBuildingFile>(File.ReadAllText(path));
            }
            catch (Exception exception)
            {
                Debug.LogError("ARGOS buildings rejected: " + exception.Message);
                LogCounts(0, 0, 0, 0);
                return 0;
            }

            if (file == null || file.buildings == null)
            {
                Debug.LogError("ARGOS buildings file has no footprints.");
                LogCounts(0, 0, 0, 0);
                return 0;
            }

            var material = ArgosShaders.Create(
                "ArgosCampusBuilding",
                "Argos/CampusBuilding",
                "Unlit/Transparent",
                "Sprites/Default");
            material.renderQueue = 2800;
            material.SetColor("_Color", new Color(0.50f, 0.56f, 0.62f, 0.62f));

            var root = new GameObject("campus-buildings");
            root.transform.SetParent(parent, false);
            var built = 0;
            for (var i = 0; i < file.buildings.Length; i++)
            {
                if (BuildOne(file.buildings[i], terrain, material, root.transform))
                    built++;
            }

            LogCounts(built, file.withHeight, file.withLevels, file.withDefault);
            return built;
        }

        static void LogCounts(int built, int withHeight, int withLevels, int withDefault)
        {
            Debug.Log(string.Format(
                CultureInfo.InvariantCulture,
                "ARGOS buildings: {0} height={1} levels={2} default={3}",
                built,
                withHeight,
                withLevels,
                withDefault));
        }

        static bool BuildOne(CampusBuildingDto building, CampusTerrain terrain, Material material, Transform parent)
        {
            if (building == null || building.outline == null || building.outline.Length < 3)
                return false;

            var ring = new List<Vector2>(building.outline.Length);
            for (var i = 0; i < building.outline.Length; i++)
            {
                var point = building.outline[i];
                if (point == null)
                    continue;
                CampusAnchor.ToLocal(point.lat, point.lon, out var north, out var west);
                var vertex = new Vector2((float)north, (float)west);
                if (ring.Count > 0 && (ring[ring.Count - 1] - vertex).sqrMagnitude < 0.01f)
                    continue;
                ring.Add(vertex);
            }

            if (ring.Count >= 2 && (ring[0] - ring[ring.Count - 1]).sqrMagnitude < 0.01f)
                ring.RemoveAt(ring.Count - 1);
            if (ring.Count < 3)
                return false;

            var roof = PolygonTriangulator.Triangulate(ring);
            if (roof == null || roof.Length < 3)
                return false;

            var count = ring.Count;
            var vertices = new Vector3[count * 2];
            var height = building.heightMeters > 0.5f ? building.heightMeters : BuildingHeight.DefaultMeters;
            for (var i = 0; i < count; i++)
            {
                var x = ring[i].x;
                var z = ring[i].y;
                var ground = terrain != null ? terrain.Sample(x, z) : 0f;
                var baseY = ground + 0.12f;
                vertices[i] = new Vector3(x, baseY, z);
                vertices[count + i] = new Vector3(x, baseY + height, z);
            }

            var indices = new List<int>(roof.Length + count * 6);
            for (var i = 0; i < roof.Length; i++)
                indices.Add(roof[i] + count);
            for (var i = 0; i < count; i++)
            {
                var next = (i + 1) % count;
                indices.Add(i);
                indices.Add(count + i);
                indices.Add(next);
                indices.Add(next);
                indices.Add(count + i);
                indices.Add(count + next);
            }

            var mesh = new Mesh
            {
                name = string.IsNullOrEmpty(building.name) ? building.osmId : building.name,
            };
            if (vertices.Length > 65000)
                mesh.indexFormat = IndexFormat.UInt32;
            mesh.vertices = vertices;
            mesh.triangles = indices.ToArray();
            mesh.RecalculateBounds();

            var go = new GameObject(mesh.name);
            go.transform.SetParent(parent, false);
            go.AddComponent<MeshFilter>().sharedMesh = mesh;
            var renderer = go.AddComponent<MeshRenderer>();
            renderer.sharedMaterial = material;
            renderer.shadowCastingMode = ShadowCastingMode.Off;
            renderer.receiveShadows = false;
            return true;
        }
    }
}
