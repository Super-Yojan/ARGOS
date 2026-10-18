using System.Collections.Generic;
using UnityEngine;

namespace Argos.Fleet
{
    public static class PathDasher
    {
        public static Vector3[][] BuildDashes(IReadOnlyList<Vector3> polyline, float dashLength, float gapLength)
        {
            if (polyline == null || polyline.Count < 2 || dashLength <= 0f || gapLength < 0f)
                return new Vector3[0][];

            var dashes = new List<Vector3[]>();
            var current = new List<Vector3>();
            var drawing = true;
            var remaining = dashLength;
            var cursor = polyline[0];
            current.Add(cursor);

            for (var i = 1; i < polyline.Count; i++)
            {
                var end = polyline[i];
                var segment = end - cursor;
                var segmentLength = segment.magnitude;
                if (segmentLength < 0.0001f)
                {
                    cursor = end;
                    continue;
                }

                var direction = segment / segmentLength;
                var travelled = 0f;
                while (travelled < segmentLength - 0.0001f)
                {
                    var step = Mathf.Min(remaining, segmentLength - travelled);
                    cursor += direction * step;
                    travelled += step;
                    remaining -= step;
                    if (drawing)
                        current.Add(cursor);

                    if (remaining > 0.0001f)
                        continue;

                    if (drawing && current.Count >= 2)
                        dashes.Add(current.ToArray());

                    current = new List<Vector3>();
                    drawing = !drawing;
                    remaining = drawing ? dashLength : gapLength;
                    if (drawing)
                        current.Add(cursor);
                }
            }

            if (drawing && current.Count >= 2)
                dashes.Add(current.ToArray());

            return dashes.ToArray();
        }
    }
}
