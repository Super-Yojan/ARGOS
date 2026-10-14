using System;
using System.IO;
using UnityEngine;
using UnityEngine.Rendering;

namespace Argos.Fleet
{
    /// <summary>
    /// Offline GMU Fairfax patch. Elevation is the bundled Terrarium tile.
    /// The photo stays dim and slightly desaturated so status colors read first.
    /// </summary>
    public sealed class CampusTerrain
    {
        public const int Zoom = 15;
        public const int TileX = 9347;
        public const int TileY = 12543;

        readonly int _width;
        readonly int _height;
        readonly float[] _relative;

        CampusTerrain(int width, int height, float[] relative)
        {
            _width = width;
            _height = height;
            _relative = relative;
        }

        public static CampusTerrain TryCreate(Transform parent)
        {
            try
            {
                var root = Path.Combine(Application.streamingAssetsPath, "terrain");
                var elevationPath = Path.Combine(root, "gmu-fairfax-terrarium.png");
                var imageryPath = Path.Combine(root, "gmu-fairfax-naip.png");
                if (!File.Exists(elevationPath))
                {
                    Debug.LogError("ARGOS terrain tile missing: " + elevationPath);
                    return null;
                }

                var elevation = LoadTexture(File.ReadAllBytes(elevationPath), mipmaps: false);
                var pixels = elevation.GetPixels32();
                var width = elevation.width;
                var height = elevation.height;
                var absolute = new float[width * height];
                for (var i = 0; i < pixels.Length; i++)
                    absolute[i] = TerrariumEncoding.Meters(pixels[i].r, pixels[i].g, pixels[i].b);

                var anchorColumn = NearestColumn(width);
                var anchorRow = NearestRow(height);
                var anchorElevation = absolute[anchorRow * width + anchorColumn];
                var relative = new float[absolute.Length];
                for (var i = 0; i < absolute.Length; i++)
                    relative[i] = absolute[i] - anchorElevation;

                var terrain = new CampusTerrain(width, height, relative);
                terrain.BuildMesh(parent, elevation, imageryPath, anchorElevation);
                UnityEngine.Object.Destroy(elevation);
                return terrain;
            }
            catch (Exception exception)
            {
                Debug.LogError("ARGOS terrain failed: " + exception.Message);
                return null;
            }
        }

        public float Sample(float unityX, float unityZ)
        {
            CampusAnchor.ToGeographic(unityX, unityZ, out var latitude, out var longitude);
            var tileX = SlippyTile.MercatorTileX(Zoom, longitude);
            var tileY = SlippyTile.MercatorTileY(Zoom, latitude);
            if (tileX < TileX || tileX >= TileX + 1d || tileY < TileY || tileY >= TileY + 1d)
                return 0f;

            var column = (tileX - TileX) * _width - 0.5d;
            var row = (1d - (tileY - TileY)) * _height - 0.5d;
            return Bilinear(column, row);
        }

        void BuildMesh(Transform parent, Texture2D elevation, string imageryPath, float anchorElevation)
        {
            var width = elevation.width;
            var height = elevation.height;
            var vertices = new Vector3[width * height];
            var uvs = new Vector2[width * height];
            for (var row = 0; row < height; row++)
            {
                for (var col = 0; col < width; col++)
                {
                    var u = (col + 0.5d) / width;
                    var vFromSouth = (row + 0.5d) / height;
                    var latitude = SlippyTile.Latitude(Zoom, TileY + 1d - vFromSouth);
                    var longitude = SlippyTile.Longitude(Zoom, TileX + u);
                    CampusAnchor.ToLocal(latitude, longitude, out var north, out var west);
                    var index = row * width + col;
                    vertices[index] = new Vector3((float)north, _relative[index], (float)west);
                    uvs[index] = new Vector2((float)u, (float)vFromSouth);
                }
            }

            var quadsX = width - 1;
            var quadsY = height - 1;
            var triangles = new int[quadsX * quadsY * 6];
            var triangle = 0;
            for (var row = 0; row < quadsY; row++)
            {
                for (var col = 0; col < quadsX; col++)
                {
                    var a = row * width + col;
                    var b = a + 1;
                    var d = a + width;
                    var c = d + 1;
                    triangles[triangle++] = a;
                    triangles[triangle++] = d;
                    triangles[triangle++] = b;
                    triangles[triangle++] = b;
                    triangles[triangle++] = d;
                    triangles[triangle++] = c;
                }
            }

            var mesh = new Mesh { name = "gmu-fairfax", indexFormat = IndexFormat.UInt32 };
            mesh.vertices = vertices;
            mesh.uv = uvs;
            mesh.triangles = triangles;
            mesh.RecalculateBounds();

            var ground = new GameObject("gmu-fairfax");
            ground.transform.SetParent(parent, false);
            var filter = ground.AddComponent<MeshFilter>();
            filter.sharedMesh = mesh;
            var renderer = ground.AddComponent<MeshRenderer>();
            renderer.sharedMaterial = CreateMaterial(imageryPath);
            renderer.shadowCastingMode = ShadowCastingMode.Off;
            renderer.receiveShadows = false;

            Debug.Log(string.Format(
                System.Globalization.CultureInfo.InvariantCulture,
                "ARGOS terrain: GMU Fairfax anchor {0},{1} tile {2}/{3}/{4} anchorElevation={5:0.0}m",
                CampusAnchor.Latitude,
                CampusAnchor.Longitude,
                Zoom,
                TileX,
                TileY,
                anchorElevation));
        }

        static Material CreateMaterial(string imageryPath)
        {
            var material = ArgosShaders.Create(
                "ArgosTerrainBasemap",
                "Argos/TerrainBasemap",
                "Unlit/Texture");
            if (File.Exists(imageryPath))
            {
                var imagery = LoadTexture(File.ReadAllBytes(imageryPath), mipmaps: true);
                imagery.wrapMode = TextureWrapMode.Clamp;
                imagery.filterMode = FilterMode.Trilinear;
                imagery.anisoLevel = 8;
                material.mainTexture = imagery;
            }
            if (material.HasProperty("_Tint"))
                material.SetColor("_Tint", new Color(0.78f, 0.80f, 0.82f, 1f));
            if (material.HasProperty("_Saturation"))
                material.SetFloat("_Saturation", 0.75f);
            return material;
        }

        static Texture2D LoadTexture(byte[] bytes, bool mipmaps)
        {
            var texture = new Texture2D(2, 2, TextureFormat.RGBA32, mipmaps);
            texture.LoadImage(bytes);
            return texture;
        }

        static int NearestColumn(int width)
        {
            var tileX = SlippyTile.MercatorTileX(Zoom, CampusAnchor.Longitude);
            var column = (int)Math.Round((tileX - TileX) * width - 0.5d);
            return Math.Max(0, Math.Min(width - 1, column));
        }

        static int NearestRow(int height)
        {
            var tileY = SlippyTile.MercatorTileY(Zoom, CampusAnchor.Latitude);
            var fromSouth = 1d - (tileY - TileY);
            var row = (int)Math.Round(fromSouth * height - 0.5d);
            return Math.Max(0, Math.Min(height - 1, row));
        }

        float Bilinear(double column, double row)
        {
            var x0 = (int)Math.Floor(column);
            var y0 = (int)Math.Floor(row);
            var tx = (float)(column - x0);
            var ty = (float)(row - y0);
            x0 = Math.Max(0, Math.Min(_width - 1, x0));
            y0 = Math.Max(0, Math.Min(_height - 1, y0));
            var x1 = Math.Min(_width - 1, x0 + 1);
            var y1 = Math.Min(_height - 1, y0 + 1);
            var a = _relative[y0 * _width + x0];
            var b = _relative[y0 * _width + x1];
            var c = _relative[y1 * _width + x0];
            var d = _relative[y1 * _width + x1];
            var ab = a + (b - a) * tx;
            var cd = c + (d - c) * tx;
            return ab + (cd - ab) * ty;
        }
    }
}
