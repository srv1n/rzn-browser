# EUI R1a Tusker friction

- `automation plan` returned `do_not_dispatch` because the project is unregistered and the daily automation budget is open, even though a human explicitly assigned in-place work. The packet path therefore could not be used as the operator skill's default loop suggests.
- EUI-T-0001/0002/0003 verification rows use direct `cargo` commands and historically underscored package names, while repo policy requires `make` and actual crate names are hyphenated. Proof was recorded with the executed Makefile-wrapped commands; task templates should inherit repo build policy and package names.
