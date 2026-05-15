# ME_OpenDRT Cube Viewer

This folder contains the experimental companion cube viewer used by the OFX plugin.

It began as a transport stub, but the current code includes a real GLFW-based viewer path:

- named pipe transport on Windows
- Unix socket transport on macOS/Linux
- identity cube and input-image cloud source modes
- OpenGL presentation on non-Apple builds
- CUDA/GL interop mesh generation where CUDA is enabled
- Metal presentation and mesh support on macOS
- CPU fallback and GPU demotion paths when runtime validation fails

## Build

The viewer is built from the top-level CMake project when enabled:

```text
-DME_OPENDRT_BUILD_CUBE_VIEWER_STUB=ON
```

The output binary is staged beside the OFX plugin where possible:

```text
ME_OpenDRT_CubeViewer
ME_OpenDRT_CubeViewer.exe
```

On macOS, CMake also stages a `ME_OpenDRT_CubeViewer.app` bundle under the OFX bundle resources.

## Runtime

The OFX plugin launches or contacts the viewer through:

```text
ME_OPENDRT_CUBE_VIEWER_EXE=<path>
ME_OPENDRT_CUBE_VIEWER_PIPE=<path-or-pipe-name>
```

If no overrides are provided, the plugin searches the OFX bundle, nearby resource directories, and finally PATH.

The viewer accepts best-effort JSON messages from the plugin, including session hello/open, parameter snapshots/deltas, input-cloud payloads, heartbeat, and close-session messages.

Development rule: the viewer must never block or destabilize the render thread. Keep transport probing and launch behavior off hot render paths.
