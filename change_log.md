# Change Log

## Mainline

- Added composer automation for TXT line tasks and top-level MD directory tasks.
- Added optional prompt TXT injection before each automated submission.
- Added automation export controls with output file naming that can reuse the source MD file name.
- Limited automation export content to the last assistant message for each completed task.
- Added per-task timeout and post-completion pause configuration.
- Added a task status table with fixed height and vertical scrolling.

## Environment Adaptation

- Switched `doctor` scripts to the Node-based cross-platform entrypoint so Windows no longer depends on `sh`.
- Added a Windows Tauri runner wrapper to keep local dev/build commands working with Windows-specific config.
- Added local Windows Tauri build override to skip updater artifact signing when no private signing key is present.
- Added Tauri file commands and frontend wrappers for local text file reads, directory MD imports, and export writes.

## Validation

- `npm run typecheck`
- `npm run test -- src/features/composer/hooks/useAutoTaskRunner.test.tsx src/features/composer/components/ComposerSend.test.tsx`
