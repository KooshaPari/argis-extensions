# Implementation Strategy

Keep the existing Taskfile structure because it already matches the requested task shape.

Refinement:
- Add a default task so `task` resolves to the main build entrypoint.
- Make `clean` switch on the detected language instead of using one Go-only cleanup path.
- Keep the existing language detection logic, since it already maps repo manifests to Go, Python, and Node.
