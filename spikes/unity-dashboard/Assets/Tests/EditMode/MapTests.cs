using System;
using System.IO;
using NUnit.Framework;
using UnityEngine;

namespace Argos.Fleet.Tests
{
    public sealed class MapTests
    {
        [Test]
        public void GmuAnchorMatchesTheArgosProjection()
        {
            CampusAnchor.ToGeographic(12d, 4d, out var latitude, out var longitude);
            Assert.Greater(latitude, CampusAnchor.Latitude);
            Assert.Less(longitude, CampusAnchor.Longitude);
            CampusAnchor.ToLocal(latitude, longitude, out var x, out var y);
            Assert.AreEqual(12d, x, 0.001d);
            Assert.AreEqual(4d, y, 0.001d);
        }

        [Test]
        public void NorthMetersIncreaseLatitudeAndWestMetersDecreaseLongitude()
        {
            CampusAnchor.ToGeographic(0d, 0d, out var latitude, out var longitude);
            Assert.AreEqual(CampusAnchor.Latitude, latitude, 1e-9);
            Assert.AreEqual(CampusAnchor.Longitude, longitude, 1e-9);
        }

        [Test]
        public void AnchorSitsInsideTheBundledTerrariumTile()
        {
            var tileX = SlippyTile.MercatorTileX(CampusTerrain.Zoom, CampusAnchor.Longitude);
            var tileY = SlippyTile.MercatorTileY(CampusTerrain.Zoom, CampusAnchor.Latitude);
            Assert.GreaterOrEqual(tileX, CampusTerrain.TileX);
            Assert.Less(tileX, CampusTerrain.TileX + 1d);
            Assert.GreaterOrEqual(tileY, CampusTerrain.TileY);
            Assert.Less(tileY, CampusTerrain.TileY + 1d);
        }

        [Test]
        public void TerrariumEncodingDecodesAKnownHeight()
        {
            Assert.AreEqual(140f, TerrariumEncoding.Meters(128, 140, 0), 1e-4f);
        }

        [Test]
        public void BundledManifestUsesTheArgosAnchorAndTile()
        {
            var path = Path.Combine(Application.streamingAssetsPath, "terrain", "gmu-fairfax.json");
            var manifest = JsonUtility.FromJson<GmuTerrainManifestDto>(File.ReadAllText(path));
            Assert.AreEqual(CampusAnchor.Latitude, manifest.anchorLatitude, 1e-9);
            Assert.AreEqual(CampusAnchor.Longitude, manifest.anchorLongitude, 1e-9);
            Assert.AreEqual(CampusTerrain.Zoom, manifest.zoom);
            Assert.AreEqual(CampusTerrain.TileX, manifest.tileX);
            Assert.AreEqual(CampusTerrain.TileY, manifest.tileY);
        }

        [Test]
        public void OccupancyGridPlacesCellsInLocalMetresAndExpiresAtHalfASecond()
        {
            const string json =
                "{\"id\":\"terra-01\",\"roverId\":1,\"sequence\":4,\"runId\":\"spike\",\"age\":0.2," +
                "\"width\":2,\"height\":2,\"resolution\":0.5,\"originX\":-1,\"originY\":3,\"occupancy\":[-1,0,65,100]}";
            var grid = OccupancyMapParser.ParseGrid(json);
            Assert.AreEqual("terra-01", grid.id);
            Assert.AreEqual(-1, OccupancyMapParser.Cell(grid, 0, 0));
            Assert.AreEqual(0, OccupancyMapParser.Cell(grid, 1, 0));
            Assert.AreEqual(65, OccupancyMapParser.Cell(grid, 0, 1));
            Assert.AreEqual(100, OccupancyMapParser.Cell(grid, 1, 1));
            Assert.AreEqual(new Vector3(-0.75f, 0f, 3.25f), OccupancyMapParser.CellCenter(grid, 0, 0));
            Assert.IsTrue(OccupancyMapParser.IsFresh(0.2d));
            Assert.IsTrue(OccupancyMapParser.IsFresh(0.499d));
            Assert.IsFalse(OccupancyMapParser.IsFresh(0.5d));
        }

        [Test]
        public void MapIndexCarriesTheVersionWithoutRequiringCells()
        {
            const string json =
                "{\"maps\":[{\"id\":\"terra-02\",\"roverId\":2,\"sequence\":9,\"runId\":\"spike\",\"age\":0.1," +
                "\"width\":16,\"height\":16,\"resolution\":0.5,\"originX\":1.5,\"originY\":-2}]}";
            var index = OccupancyMapParser.ParseIndex(json);
            Assert.AreEqual(1, index.maps.Length);
            Assert.AreEqual("terra-02", index.maps[0].id);
            Assert.AreEqual(9d, index.maps[0].sequence);
            Assert.AreEqual("spike", index.maps[0].runId);
            Assert.IsNull(index.maps[0].occupancy);
            Assert.IsTrue(OccupancyMapParser.IsFresh(index.maps[0].age));
        }

        [Test]
        public void OccupancyColorsSeparateUnknownFreeAndOccupied()
        {
            var unknown = OccupancyStyle.ColorFor(-1);
            var free = OccupancyStyle.ColorFor(0);
            var occupied = OccupancyStyle.ColorFor(90);
            Assert.AreEqual(0, unknown.a);
            Assert.Greater(free.a, 0);
            Assert.Less(free.a, 80);
            Assert.Greater(occupied.a, 200);
            Assert.Less(occupied.r, free.r);
        }
    }

    [Serializable]
    public sealed class GmuTerrainManifestDto
    {
        public double anchorLatitude;
        public double anchorLongitude;
        public int zoom;
        public int tileX;
        public int tileY;
    }
}
