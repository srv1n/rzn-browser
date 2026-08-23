# Xcode Build-State Doctor

Use this when an `xcodebuild` proof command fails in a way that may be generated
Xcode infrastructure instead of source code.

## Command

```bash
tusker xcode doctor --project <App.xcodeproj> --log <build.log> --dry-run
tusker xcode doctor --workspace <App.xcworkspace> --result-bundle <Build.xcresult> --dry-run
tusker xcode doctor --project <App.xcodeproj> --cleanup
```

## Classifications

- `likely_infrastructure`: generated Xcode build state is probably corrupt, such
  as stale `build.db` locks, build database I/O/internal inconsistency, or
  `supplementaryOutputs` map corruption.
- `likely_code`: the logs look like compiler, linker, or module failures.
- `unknown`: Tusker did not find a known signature.

## Cleanup Rule

Run dry-run first. Cleanup may remove only generated `DerivedData/**/XCBuildData`
paths scoped to the explicit project or workspace name. Broad paths such as the
global DerivedData root must be refused.

## Proof Rule

Do not record this as build-green proof. If the doctor reports
`likely_infrastructure`, record the doctor output as blocked infrastructure proof,
run scoped cleanup if appropriate, and rerun the original `xcodebuild` command.
Only the rerun can prove the code builds.
