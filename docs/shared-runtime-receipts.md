# Shared runtime vehicle feedback

The GPUI dashboard continues using its current drive controls and streamed relay
status. A published level change now says it was requested and awaits a vehicle
acknowledgement. The fleet poll consumes the vehicle's token-correlated authority
receipt and displays acceptance or the vehicle's explicit rejection reason.
Receipts from before the current polling session are ignored, and unchanged
receipts do not repeatedly overwrite the dashboard message. This is presentation
of the shared runtime's decision, not independent dashboard command admission.

Current relay-status fixtures use ready/output_active/fault; removed hardware
arm requests are rejected. Exploration statuses remain visible as supported
vehicle authority values. The Pi actuator ID and -100..100 effort protocol is
unchanged. Full ARGOS workspace verification: 152 passed, zero failed, six GPU/
live-Google tests skipped. Desktop workspace rebuilt successfully.
