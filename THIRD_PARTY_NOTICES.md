# Third-party notices

Snapline is an independent native Windows implementation inspired by **Tendedero**, by Alejandro Buján: https://github.com/alejandrobujan/tendedero . The original MIT copyright notice and name/icon restrictions are retained in LICENSE. The original application's name, icon artwork, and promotional images are not included.

## Slint

Slint UI toolkit, copyright SixtyFPS GmbH and contributors: https://slint.dev . Snapline uses Slint under the **Slint Royalty-free Desktop, Mobile, and Web Applications License, version 2.0**. The complete text is included in SLINT_LICENSE.md. Attribution is provided by the AboutSlint widget in the application's About dialog, accessible from Settings and the tray menu.

## Rust dependencies

The application also uses these libraries, each under its own license:

| Library | Purpose | License |
| --- | --- | --- |
| windows | Native Windows API bindings | MIT / Apache-2.0 |
| raw-window-handle | Native window handles | MIT / Apache-2.0 / Zlib |
| image | Image decoding and encoding | MIT / Apache-2.0 |
| arboard | Clipboard access | MIT / Apache-2.0 |
| serde, serde_json | Local settings and data serialization | MIT / Apache-2.0 |
| sha2 | Duplicate image detection | MIT / Apache-2.0 |
| tray-icon | Windows notification-area icon | MIT / Apache-2.0 |
| global-hotkey | Global keyboard shortcut | MIT / Apache-2.0 |
| rfd | Native file dialogs | MIT |
| trash | Windows recycle-bin operations | MIT |
| winresource | Executable icon and manifest resources | MIT |

Cargo.lock records the exact dependency versions. Dependency license metadata and available license/copyright notices are collected in THIRD_PARTY_LICENSES.txt in release packages. The application license does not replace these dependency licenses.
