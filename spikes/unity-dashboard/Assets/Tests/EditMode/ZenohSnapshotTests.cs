using NUnit.Framework;
using UnityEngine;

namespace Argos.Fleet.Tests
{
    public sealed class ZenohSnapshotTests
    {
        const string Snapshot =
            "{\"snapshotUnixSeconds\":1000,\"link\":\"healthy\",\"rovers\":[" +
            "{\"id\":\"terra-01\",\"position\":{\"x\":1,\"y\":0,\"z\":2},\"headingDegrees\":90,\"status\":\"normal\",\"lastUpdateUnixSeconds\":999.9}," +
            "{\"id\":\"terra-02\",\"position\":{\"x\":-4.6,\"y\":0,\"z\":-6.6},\"headingDegrees\":10,\"status\":\"attention\",\"lastUpdateUnixSeconds\":999.8}," +
            "{\"id\":\"terra-03\",\"position\":{\"x\":0,\"y\":0,\"z\":-6.6},\"headingDegrees\":20,\"status\":\"critical\",\"lastUpdateUnixSeconds\":999.7}," +
            "{\"id\":\"terra-04\",\"position\":{\"x\":4,\"y\":0,\"z\":-6},\"headingDegrees\":0,\"status\":\"critical\",\"lastUpdateUnixSeconds\":997.4}" +
            "]}";

        [Test]
        public void BridgeSnapshotFeedsTheExistingStatusAndStalePresentation()
        {
            var snapshot = TelemetryParser.Parse(Snapshot);
            Assert.AreEqual(1000d, snapshot.snapshotUnixSeconds);
            Assert.AreEqual(4, snapshot.rovers.Length);

            var normal = TelemetryParser.Evaluate(snapshot.rovers[0], snapshot.snapshotUnixSeconds);
            Assert.AreEqual("terra-01", normal.Id);
            Assert.AreEqual(new Vector3(1f, 0f, 2f), normal.Position);
            Assert.AreEqual(90f, normal.HeadingDegrees);
            Assert.AreEqual(RoverStatus.Normal, normal.Status);
            Assert.IsFalse(normal.IsStale);
            Assert.Less(normal.AgeSeconds, TelemetryRules.StaleAfterSeconds);

            var attention = TelemetryParser.Evaluate(snapshot.rovers[1], snapshot.snapshotUnixSeconds);
            Assert.AreEqual(RoverStatus.Attention, attention.Status);
            Assert.IsFalse(attention.IsStale);
            var amber = StatusLanguage.Resolve(attention.Status, attention.IsStale, dimmed: false);
            Assert.Greater(amber.r, 0.9f);
            Assert.Greater(amber.g, 0.4f);
            Assert.Less(amber.b, 0.3f);

            var critical = TelemetryParser.Evaluate(snapshot.rovers[2], snapshot.snapshotUnixSeconds);
            Assert.AreEqual(RoverStatus.Critical, critical.Status);
            Assert.IsFalse(critical.IsStale);
            var red = StatusLanguage.Resolve(critical.Status, critical.IsStale, dimmed: false);
            Assert.Greater(red.r, 0.8f);
            Assert.Less(red.g, 0.3f);

            var ghost = TelemetryParser.Evaluate(snapshot.rovers[3], snapshot.snapshotUnixSeconds);
            Assert.IsTrue(ghost.IsStale);
            Assert.Greater(ghost.AgeSeconds, TelemetryRules.StaleAfterSeconds);
            Assert.AreEqual(RoverStatus.Critical, ghost.Status);
            var ghosted = StatusLanguage.Resolve(ghost.Status, ghost.IsStale, dimmed: false);
            Assert.Less(ghosted.a, 0.5f);
        }
    }
}
