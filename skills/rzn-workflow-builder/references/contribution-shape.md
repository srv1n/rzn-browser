# Contribution shape

A reviewer should be able to identify the capability, required inputs, returned
data, side effects, and a runnable example from the workflow's inspected contract.

- Put production JSON in `workflows/<system>/`; keep selectors and DOM logic there.
- Update the pack's existing documentation when usage changes. Add a separate
  workflow document only when it explains something the manifest help cannot.
- Include focused validation and evidence for changed runtime behavior, with
  any missing live check stated explicitly.
- Keep generated probes out of production packs until they have a stable contract.

Contributing a workflow does not require installing the runtime globally,
publishing a release, or creating a document for every file.
