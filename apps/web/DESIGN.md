# ARGOS fleet field

The fleet is the interface. Show every rover at the same time on one shared field. Do not add a roster, vehicle list, command table, or per-rover page as the primary navigation.

Select a rover directly on the field to open a dismissible inspection and waypoint panel. Selection never hides the rest of the fleet. Separate overlapping callouts in screen space and connect them to the original reported positions, so co-located vehicles remain individually visible and selectable. Start with no selection. Keep telemetry, mode and control ownership in a compact toolbar so the field fills the screen.

Fit fleet frames all positioned vehicles. Align independent phone frames only through reported geographic references; explicitly shared local frame IDs may share a local plot. Unknown or unaligned vehicles stay visible in a labelled, non-spatial area instead of receiving invented coordinates. Map placement converts back to the selected rover's original coordinate frame before command confirmation.

Retain the workspace's paper-and-ink tokens, keyboard-accessible markers, responsive controls, explicit command confirmation and reduced-motion behavior. Backend command policy and persistent audit records remain authoritative even though the overview does not display them as a list.
