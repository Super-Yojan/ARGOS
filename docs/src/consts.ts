export const SITE_TITLE = "ARGOS";

/** Prefix an in-site path with the project Page base, `/ARGOS`. */
export const href = (path: string) =>
    `${import.meta.env.BASE_URL.replace(/\/$/, "")}${path === "/" ? "/" : path}`;

/** Every page, in reading order. The menu and the index use this list. */
export const pages = [
    { path: "/running", label: "Download and run", blurb: "Get the dashboard for macOS, Windows or Linux, and open it." },
    { path: "/modes", label: "Simulation or real", blurb: "Point ARGOS at a local Zorvane simulator or at the real fleet." },
    { path: "/operating", label: "Operate a rover", blurb: "Select, take control, arm, drive, send goals and choose an autonomy level." },
    { path: "/agents", label: "Agents", blurb: "Let Claude or any MCP agent advise you once a minute, or operate the fleet through the same checks." },
    { path: "/video", label: "FPV video", blurb: "Watch a rover's front camera over WebRTC." },
    { path: "/settings", label: "Settings", blurb: "Every variable ARGOS reads from .env or the environment." },
];
