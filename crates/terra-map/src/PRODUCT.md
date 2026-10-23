# Native 3D map viewer

A native Rust/GPUI application for exploring Google Photorealistic 3D Tiles.
The primary task is navigating a textured 3D neighborhood and inspecting camera
and rendering state. Google API credentials are supplied locally at launch.

The user explicitly chose native Rust and supplied a visual reference showing
dark, cinematic blue-green city maps with translucent HUD panels and cyan accents.
That reference supersedes the initial black-and-white application chrome.

Assumptions: desktop mouse/keyboard operation; San Francisco is the initial
location; controls expose real camera/rendering state. No analytics dataset was
provided, so decorative traffic-flow lines and fictional charts are excluded.
