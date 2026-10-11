using UnityEngine;

namespace Argos.Fleet
{
    /// <summary>
    /// Rough stand-in for the design's status language: cyan / amber / red, ghosted when stale,
    /// dimmed when some other rover is selected.
    /// </summary>
    public static class StatusLanguage
    {
        public static readonly Color Cyan = new Color(0.15f, 0.95f, 1f, 1f);
        public static readonly Color Amber = new Color(1f, 0.62f, 0.08f, 1f);
        public static readonly Color Red = new Color(1f, 0.18f, 0.12f, 1f);
        public static readonly Color Ghost = new Color(0.62f, 0.68f, 0.72f, 0.28f);

        public static Color Resolve(RoverStatus status, bool stale, bool dimmed)
        {
            Color color;
            if (stale)
            {
                color = Ghost;
            }
            else
            {
                switch (status)
                {
                    case RoverStatus.Attention:
                        color = Amber;
                        break;
                    case RoverStatus.Critical:
                        color = Red;
                        break;
                    default:
                        color = Cyan;
                        break;
                }
            }

            if (!dimmed)
                return color;

            color.r *= 0.35f;
            color.g *= 0.35f;
            color.b *= 0.35f;
            color.a *= 0.35f;
            return color;
        }
    }
}
