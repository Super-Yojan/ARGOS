using System;
using UnityEngine;

namespace Argos.Fleet
{
    /// <summary>
    /// Occupancy overlays match the Swift frame: a snapshot is fresh only while age is under 0.5 s.
    /// Cells are row-major, +X columns and +Y rows, in the same local metres as pose.
    /// </summary>
    public static class OccupancyRules
    {
        public const double FreshBelowSeconds = 0.5;
    }

    [Serializable]
    public sealed class OccupancyIndexDto
    {
        public OccupancyMapDto[] maps;
    }

    [Serializable]
    public sealed class OccupancyMapDto
    {
        public string id;
        public double roverId;
        public double sequence;
        public string runId;
        public double age;
        public int width;
        public int height;
        public float resolution;
        public float originX;
        public float originY;
        public int[] occupancy;
    }

    public static class OccupancyMapParser
    {
        public static OccupancyIndexDto ParseIndex(string json)
        {
            if (string.IsNullOrWhiteSpace(json))
                throw new ArgumentException("Occupancy index JSON is empty.", nameof(json));

            var index = JsonUtility.FromJson<OccupancyIndexDto>(json);
            if (index == null || index.maps == null)
                throw new ArgumentException("Occupancy index JSON did not contain maps.", nameof(json));

            return index;
        }

        public static OccupancyMapDto ParseGrid(string json)
        {
            if (string.IsNullOrWhiteSpace(json))
                throw new ArgumentException("Occupancy grid JSON is empty.", nameof(json));

            var grid = JsonUtility.FromJson<OccupancyMapDto>(json);
            if (grid == null || grid.occupancy == null)
                throw new ArgumentException("Occupancy grid JSON did not contain cells.", nameof(json));
            if (grid.width <= 0 || grid.height <= 0 || grid.occupancy.Length != grid.width * grid.height)
                throw new ArgumentException("Occupancy grid dimensions do not match the cell list.", nameof(json));

            return grid;
        }

        public static bool IsFresh(double ageSeconds)
        {
            if (double.IsNaN(ageSeconds))
                return false;
            return ageSeconds >= 0d && ageSeconds < OccupancyRules.FreshBelowSeconds;
        }

        public static int Cell(OccupancyMapDto grid, int column, int row)
        {
            if (grid == null)
                throw new ArgumentNullException(nameof(grid));
            if (column < 0 || row < 0 || column >= grid.width || row >= grid.height)
                throw new ArgumentOutOfRangeException(nameof(column));
            return grid.occupancy[row * grid.width + column];
        }

        /// <summary>
        /// Cell centre in Unity metres. Local +x (north) is Unity x. Local +y (west) is Unity z.
        /// </summary>
        public static Vector3 CellCenter(OccupancyMapDto grid, int column, int row)
        {
            if (grid == null)
                throw new ArgumentNullException(nameof(grid));
            return new Vector3(
                grid.originX + (column + 0.5f) * grid.resolution,
                0f,
                grid.originY + (row + 0.5f) * grid.resolution);
        }
    }

    /// <summary>
    /// Unknown is clear, free space is a faint tint, occupied cells are dark and solid.
    /// </summary>
    public static class OccupancyStyle
    {
        public static Color32 ColorFor(int cell)
        {
            if (cell < 0)
                return new Color32(0, 0, 0, 0);
            if (cell == 0)
                return new Color32(148, 186, 196, 40);

            var t = Mathf.Clamp01(cell / 100f);
            var shade = (byte)Mathf.Lerp(64f, 14f, t);
            var alpha = (byte)Mathf.Lerp(210f, 245f, t);
            return new Color32(shade, (byte)(shade + 2), (byte)(shade + 8), alpha);
        }
    }
}
