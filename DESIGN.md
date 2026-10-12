# Native screenshot clothesline

Mode: Operate. The transparent top-edge overlay is a tool on the user's existing desktop. Warm paper thumbnails hang from a thin rope with small clothespins. Motion is gentle, brief and subordinate to pointer interaction. Settings and editing use clear, legible native desktop controls with Chinese and English labels.

The overlay must not steal focus, obscure fullscreen apps by default, or block the desktop outside interactive content. Support multiple monitor positions and per-monitor scaling. Empty state explains Win+Shift+S and import. Actions are explicit and status messages describe recovery from errors.

The rope spans the current monitor from edge to edge. Its geometry uses the live window width, with a preferred startup size rather than fixed layout constraints. Display size and DPI changes refresh the overlay and thumbnail layout.

Primary surfaces: transparent clothesline; settings; image preview/editor; About with Slint attribution. Preserve reference concept, without using upstream name, icon or documentation imagery.

Image gestures follow one contract: double-click opens the built-in editor; right-click copies; dragging shares a file. Single-click has no clipboard side effect. The editor's primary Save replaces the selected owned image and its thumbnail in place; Save a copy exports a separate PNG. Imported source files remain untouched, and a failed save retains the prior cached image.

Capture success is one brief sequence: a photograph-only shutter flash and optional mechanical click, a 500ms decelerating shrink to the photograph's peg, then a small settling sway. A separate transparent native window keeps the desktop clickable and never takes focus. Only new captures animate; manual imports and existing cards stay quiet. Rapid captures replace the current flight. Windows reduced motion removes travel, flash and sway while retaining a static arrival confirmation. Clipboard images have no screen origin, so the photograph begins centered on the active monitor with its aspect ratio preserved.

Photos run oldest to newest from left to right. A new arrival opens the latest page. At the retention limit, the oldest visible photograph detaches from its peg, accelerates downwards with a slight tilt, and fades out over 520ms. Surviving photos remain in their original slots until every active fall has finished, then slide left into the latest layout over 230ms. Each falling photograph keeps its own timing during rapid arrivals; survivors move once after all falls finish. The transparent helper never activates or intercepts desktop input, and reduced motion removes the fall and slide.

Settings use a fresh, quiet palette: warm ivory content, a pale mint navigation rail, deep green text and a restrained green Save action. Four icon-led sections separate image sources, clothesline behavior, appearance and startup; editable values persist while switching sections and commit only on Save. Thin line icons share a 24px grid and 1.7px rounded stroke. Dark mode uses muted forest surfaces with the same hierarchy, and native input controls retain their keyboard behavior.
