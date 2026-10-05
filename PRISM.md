# Project notes

Things Prism should know every time it works on this project.

## Notes

- Gadget backends run as the app's own Prism.exe with ELECTRON_RUN_AS_NODE=1; if a rebuild of C:\Users\Valerie\prism-work\dist wipes icudtl.dat/snapshot_blob.bin next to the running exe, every gadget dies with "Invalid file descriptor to ICU data received" — restore the files from the complete build folder (e.g. dist\build-<version>\win-unpacked) or restart the app from the new build.
