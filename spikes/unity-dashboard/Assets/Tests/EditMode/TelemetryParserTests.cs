using System.IO;
using NUnit.Framework;
using UnityEngine;

namespace Argos.Fleet.Tests
{
    public sealed class TelemetryParserTests
    {
        [Test]
        public void StaleThresholdMatchesTheDesignWindow()
        {
            Assert.AreEqual(2.5d, TelemetryRules.StaleAfterSeconds);
        }

        [Test]
        public void RoverIsFreshAtExactlyTwoAndAHalfSeconds()
        {
            var view = TelemetryParser.Evaluate(Rover(lastUpdate: 100d), nowUnixSeconds: 102.5d);
            Assert.IsFalse(view.IsStale);
            Assert.AreEqual(2.5d, view.AgeSeconds, 1e-9);
        }

        [Test]
        public void RoverIsStaleJustAfterTwoAndAHalfSeconds()
        {
            var view = TelemetryParser.Evaluate(Rover(lastUpdate: 100d), nowUnixSeconds: 102.5d + 0.0001d);
            Assert.IsTrue(view.IsStale);
            Assert.AreEqual(RoverStatus.Normal, view.Status);
        }

        [Test]
        public void FreshAttentionStaysAmberUntilTheWindowExpires()
        {
            var view = TelemetryParser.Evaluate(Rover("attention", 50d), 52.5d);
            Assert.IsFalse(view.IsStale);
            Assert.AreEqual(RoverStatus.Attention, view.Status);
            var color = StatusLanguage.Resolve(view.Status, view.IsStale, dimmed: false);
            Assert.Greater(color.r, 0.9f);
            Assert.Greater(color.g, 0.4f);
            Assert.Less(color.b, 0.3f);
        }

        [Test]
        public void StaleRoverIsGhostedEvenWhenStatusIsCritical()
        {
            var view = TelemetryParser.Evaluate(Rover("critical", 10d), 20d);
            Assert.IsTrue(view.IsStale);
            Assert.AreEqual(RoverStatus.Critical, view.Status);
            var color = StatusLanguage.Resolve(view.Status, view.IsStale, dimmed: false);
            Assert.Less(color.a, 0.5f);
            Assert.Greater(color.r, 0.4f);
            Assert.Greater(color.g, 0.4f);
            Assert.Greater(color.b, 0.4f);
        }

        [Test]
        public void SelectionDimsUnselectedRovers()
        {
            var full = StatusLanguage.Resolve(RoverStatus.Normal, stale: false, dimmed: false);
            var dimmed = StatusLanguage.Resolve(RoverStatus.Normal, stale: false, dimmed: true);
            Assert.Less(dimmed.a, full.a);
            Assert.Less(dimmed.g, full.g);
        }

        [Test]
        public void ParserReadsIdPoseHeadingStatusAndTimestamp()
        {
            const string json = "{\"snapshotUnixSeconds\":1000,\"rovers\":[{\"id\":\"terra-01\",\"position\":{\"x\":1,\"y\":2,\"z\":3},\"headingDegrees\":90,\"status\":\"critical\",\"lastUpdateUnixSeconds\":999,\"completedPath\":[{\"x\":0,\"y\":0,\"z\":0},{\"x\":1,\"y\":0,\"z\":0}],\"plannedPath\":[{\"x\":1,\"y\":0,\"z\":0},{\"x\":2,\"y\":0,\"z\":1}]}]}";
            var snapshot = TelemetryParser.Parse(json);
            Assert.AreEqual(1000d, snapshot.snapshotUnixSeconds);
            Assert.AreEqual(1, snapshot.rovers.Length);
            var view = TelemetryParser.Evaluate(snapshot.rovers[0], snapshot.snapshotUnixSeconds);
            Assert.AreEqual("terra-01", view.Id);
            Assert.AreEqual(new Vector3(1f, 2f, 3f), view.Position);
            Assert.AreEqual(90f, view.HeadingDegrees);
            Assert.AreEqual(RoverStatus.Critical, view.Status);
            Assert.IsFalse(view.IsStale);
            Assert.AreEqual(2, snapshot.rovers[0].completedPath.Length);
            Assert.AreEqual(2, snapshot.rovers[0].plannedPath.Length);
        }

        [Test]
        public void RecordedFeedHasTwentyRoversAndTheStaleCases()
        {
            var path = Path.Combine(Application.dataPath, "StreamingAssets", "fleet-telemetry.json");
            var snapshot = TelemetryParser.Parse(File.ReadAllText(path));
            Assert.AreEqual(20, snapshot.rovers.Length);

            var stale = 0;
            var attention = 0;
            var critical = 0;
            foreach (var rover in snapshot.rovers)
            {
                var view = TelemetryParser.Evaluate(rover, snapshot.snapshotUnixSeconds);
                if (view.IsStale)
                    stale++;
                if (view.Status == RoverStatus.Attention)
                    attention++;
                if (view.Status == RoverStatus.Critical)
                    critical++;

                if (rover.id == "terra-02")
                {
                    Assert.AreEqual(2.5d, view.AgeSeconds, 1e-6);
                    Assert.IsFalse(view.IsStale);
                }

                if (rover.id == "terra-08")
                    Assert.IsTrue(view.IsStale);
            }

            Assert.GreaterOrEqual(stale, 1);
            Assert.GreaterOrEqual(attention, 1);
            Assert.GreaterOrEqual(critical, 1);
            Assert.AreEqual(3, stale);
        }

        [Test]
        public void PlannedPathDashesLeaveGaps()
        {
            var polyline = new[]
            {
                new Vector3(0f, 0f, 0f),
                new Vector3(10f, 0f, 0f),
            };
            var dashes = PathDasher.BuildDashes(polyline, dashLength: 1f, gapLength: 1f);
            Assert.AreEqual(5, dashes.Length);
            Assert.AreEqual(0f, dashes[0][0].x, 0.001f);
            Assert.Greater(dashes[1][0].x, dashes[0][1].x);
        }

        static RoverDto Rover(string status = "normal", double lastUpdate = 0d)
        {
            return new RoverDto
            {
                id = "terra-test",
                position = new Vec3Dto { x = 0f, y = 0f, z = 0f },
                headingDegrees = 0f,
                status = status,
                lastUpdateUnixSeconds = lastUpdate,
            };
        }
    }
}
