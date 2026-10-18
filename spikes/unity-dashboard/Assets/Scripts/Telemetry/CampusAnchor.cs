using System;

namespace Argos.Fleet
{
    /// <summary>
    /// GMU Fairfax origin used by ARGOS and Zorvane.
    /// +x is north, +y is west. The scale matches argos_core::Anchor and Swift MapProjection.
    /// </summary>
    public static class CampusAnchor
    {
        public const double Latitude = 38.8297;
        public const double Longitude = -77.3075;
        public const double EquatorialCircumferenceMeters = 40075016.686;

        public static double MetersPerDegree => EquatorialCircumferenceMeters / 360.0;

        public static void ToGeographic(double xNorth, double yWest, out double latitude, out double longitude)
        {
            var scale = MetersPerDegree;
            var cosine = Math.Cos(Latitude * Math.PI / 180.0);
            latitude = Latitude + xNorth / scale;
            longitude = Longitude - yWest / (scale * cosine);
        }

        public static void ToLocal(double latitude, double longitude, out double xNorth, out double yWest)
        {
            var scale = MetersPerDegree;
            var cosine = Math.Cos(Latitude * Math.PI / 180.0);
            xNorth = (latitude - Latitude) * scale;
            yWest = -(longitude - Longitude) * scale * cosine;
        }
    }

    /// <summary>
    /// Web-Mercator slippy tiles, the scheme Terrarium elevation tiles use.
    /// </summary>
    public static class SlippyTile
    {
        public static double Latitude(int zoom, double tileY)
        {
            var n = Math.Pow(2, zoom);
            return Math.Atan(Math.Sinh(Math.PI * (1 - 2 * tileY / n))) * 180.0 / Math.PI;
        }

        public static double Longitude(int zoom, double tileX)
        {
            var n = Math.Pow(2, zoom);
            return tileX / n * 360.0 - 180.0;
        }

        public static double MercatorTileY(int zoom, double latitudeDegrees)
        {
            var n = Math.Pow(2, zoom);
            var latitude = latitudeDegrees * Math.PI / 180.0;
            return (1 - Math.Log(Math.Tan(latitude) + 1 / Math.Cos(latitude)) / Math.PI) / 2 * n;
        }

        public static double MercatorTileX(int zoom, double longitudeDegrees)
        {
            var n = Math.Pow(2, zoom);
            return (longitudeDegrees + 180.0) / 360.0 * n;
        }
    }

    public static class TerrariumEncoding
    {
        public static float Meters(byte red, byte green, byte blue)
        {
            return (red * 256f + green + blue / 256f) - 32768f;
        }
    }
}
