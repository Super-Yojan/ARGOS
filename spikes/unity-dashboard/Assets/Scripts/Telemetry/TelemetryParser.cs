using System;
using UnityEngine;

namespace Argos.Fleet
{
    /// <summary>
    /// Lost-link window from the ARGOS 3D dashboard design: pose older than 2.5 s is stale.
    /// The last known pose is kept. Nothing in this parser invents a new position.
    /// </summary>
    public static class TelemetryRules
    {
        public const double StaleAfterSeconds = 2.5;
    }

    public enum RoverStatus
    {
        Unknown = 0,
        Normal = 1,
        Attention = 2,
        Critical = 3,
    }

    [Serializable]
    public sealed class Vec3Dto
    {
        public float x;
        public float y;
        public float z;

        public Vector3 ToVector3()
        {
            return new Vector3(x, y, z);
        }
    }

    [Serializable]
    public sealed class RoverDto
    {
        public string id;
        public Vec3Dto position;
        public float headingDegrees;
        public string status;
        public double lastUpdateUnixSeconds;
        public Vec3Dto[] completedPath;
        public Vec3Dto[] plannedPath;
    }

    [Serializable]
    public sealed class FleetSnapshotDto
    {
        public double snapshotUnixSeconds;
        public RoverDto[] rovers;
    }

    public readonly struct RoverView
    {
        public RoverView(string id, RoverStatus status, bool isStale, double ageSeconds, Vector3 position, float headingDegrees)
        {
            Id = id ?? string.Empty;
            Status = status;
            IsStale = isStale;
            AgeSeconds = ageSeconds;
            Position = position;
            HeadingDegrees = headingDegrees;
        }

        public string Id { get; }
        public RoverStatus Status { get; }
        public bool IsStale { get; }
        public double AgeSeconds { get; }
        public Vector3 Position { get; }
        public float HeadingDegrees { get; }
    }

    public static class TelemetryParser
    {
        public static FleetSnapshotDto Parse(string json)
        {
            if (string.IsNullOrWhiteSpace(json))
                throw new ArgumentException("Telemetry JSON is empty.", nameof(json));

            var snapshot = JsonUtility.FromJson<FleetSnapshotDto>(json);
            if (snapshot == null || snapshot.rovers == null)
                throw new ArgumentException("Telemetry JSON did not contain a rover list.", nameof(json));

            return snapshot;
        }

        public static RoverStatus ParseStatus(string status)
        {
            if (string.IsNullOrEmpty(status))
                return RoverStatus.Unknown;

            switch (status.Trim().ToLowerInvariant())
            {
                case "normal":
                case "cyan":
                    return RoverStatus.Normal;
                case "attention":
                case "amber":
                    return RoverStatus.Attention;
                case "critical":
                case "red":
                    return RoverStatus.Critical;
                default:
                    return RoverStatus.Unknown;
            }
        }

        public static bool IsStale(double lastUpdateUnixSeconds, double nowUnixSeconds)
        {
            if (double.IsNaN(lastUpdateUnixSeconds) || double.IsNaN(nowUnixSeconds))
                return true;

            return (nowUnixSeconds - lastUpdateUnixSeconds) > TelemetryRules.StaleAfterSeconds;
        }

        public static RoverView Evaluate(RoverDto rover, double nowUnixSeconds)
        {
            if (rover == null)
                throw new ArgumentNullException(nameof(rover));

            var position = rover.position != null ? rover.position.ToVector3() : Vector3.zero;
            var age = nowUnixSeconds - rover.lastUpdateUnixSeconds;
            return new RoverView(
                rover.id,
                ParseStatus(rover.status),
                IsStale(rover.lastUpdateUnixSeconds, nowUnixSeconds),
                age,
                position,
                rover.headingDegrees);
        }
    }
}
