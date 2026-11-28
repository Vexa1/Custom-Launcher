## What changed

- README structure: the Catalina FFI rules and the design-system chapter are subsections of the
  feature reference again, so sections 2.6–2.9 (memory/refresh, Party, Server hop, shell) are no
  longer nested under the design-system heading.
- New **Installing from the release page** section explains when to take the full archive
  (`Build CREBLOX.command`, fresh machine) and when to take the quick update
  (`Update CREBLOX.command`, existing source folder, settings kept), with the shared
  requirements and checksum check.
- Fixed the overlay's **Visible for** slider showing `10 s s`: the custom formatter already
  carries the unit, so the duplicate suffix is gone (`until game closes` still shows for 0).
