# Diagram symbols

Original symbols drawn for Locus (nothing copied from other products). Each file is a 100 × 100 SVG centred on the origin, drawn in `currentColor`, seen from above; `index.json` gives each symbol's name and its default size on the ground in metres.

`make_symbols.py` writes all of them: change a drawing there and run `py assets/symbols/make_symbols.py`. The editor (app/src/diagram2d/symbols.ts) and printing (crates/locus-report/src/diagram.rs) both read these files; a test checks the two lists match.
