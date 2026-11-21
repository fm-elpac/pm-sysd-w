# Windows Integration (pa / p_w / p_u)

This page covers platform abstraction and Windows integration. `unsafe` code is
in [unsafe.md](unsafe.md).

## Platform Abstraction (pa)

`pa` is the low-level platform abstraction interface, using conditional
compilation to choose the implementation: Windows uses `p_w` (main platform),
Linux uses `p_u` (for testing only). Other modules depend only on `pa`.

It abstracts two kinds of differences: process management (start / stop / read
output), and IPC connections (named pipe / UNIX socket).

## Windows Implementation (p_w)

`p_w` contains only platform-specific code, no abstraction / wrapping. IPC uses
named pipes, allowing local connections only and forbidding network access.

## Linux Implementation (p_u)

`p_u` is for convenient testing only. IPC uses a UNIX socket in the current
directory.

## Named Pipes and Permissions

Named-pipe names are decided by `pa::Instance`. The `--system` instance creates
an extra read-only pipe for read-only queries by ordinary users. Exact names and
ACL are in the `pa` / `p_w::acl` code comments.

The manual named-pipe permission test script is `test/test-permission.ps1`
(requires admin; cross-user permissions are not covered by automated tests).

## Install Script and Windows Service

`install/setup.ps1` is the experimental install / uninstall script (per-user
service template); its behavior is in the user guide and the script itself.
pm-sysd-init's Windows service (SCM) integration is in the `p_w::service` code
comments.

## Known Limitations

The install script places the exe in the current user's directory, but the
service template is machine-level (HKLM), so it currently suits a single-user
scenario only.
