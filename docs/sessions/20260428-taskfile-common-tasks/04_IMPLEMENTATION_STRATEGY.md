# Implementation Strategy

Keep the existing Taskfile structure because it already matches the requested task shape.

Refinement:
- Add a default task so `task` resolves to the main build entrypoint.
- Make `clean` switch on the detected language instead of using one Go-only cleanup path.
- Keep the existing language detection shape, but scope it to root manifests so nested subprojects do
  not override the primary repo language.
