"""Write Locus's original diagram symbols (drawn for this project) to assets/symbols/.
Each is a 100 x 100 unit SVG centred on the origin, in currentColor, seen from above."""
import pathlib, json

out = pathlib.Path(__file__).resolve().parent

S = 'fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round"'
S_BOLD = 'fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round"'
F = 'fill="currentColor"'
symbols = {
    "car": ("Car", 4.5, f'<rect x="-45" y="-20" width="90" height="40" rx="10" {S}/><path d="M 10 -17 Q 20 0 10 17 M -22 -17 Q -30 0 -22 17" {S}/>'),
    "truck": ("Truck", 8.0, f'<rect x="-46" y="-18" width="64" height="36" rx="3" {S}/><rect x="22" y="-16" width="24" height="32" rx="6" {S}/><path d="M 34 -14 L 34 14" {S}/>'),
    "motorcycle": ("Motorcycle", 2.2, f'<path d="M -44 0 L 44 0" {S_BOLD}/><path d="M 22 -14 L 22 14" {S}/><ellipse cx="-5" cy="0" rx="14" ry="8" {S}/>'),
    "person": ("Person", 0.6, f'<ellipse cx="0" cy="0" rx="42" ry="20" {S}/><circle cx="0" cy="0" r="13" {S}/>'),
    "body": ("Body outline", 1.8, f'<circle cx="-36" cy="0" r="9" {S}/><path d="M -26 -10 L 16 -12 L 44 -6 M -26 10 L 16 12 L 44 6 M -18 -10 L -8 -28 M -18 10 L -8 28 M 16 -12 L 16 12" {S}/>'),
    "blood": ("Blood", 0.3, f'<path d="M 0 -40 C 18 -10 30 6 30 18 A 30 30 0 0 1 -30 18 C -30 6 -18 -10 0 -40 Z" {F}/>'),
    "cartridge_case": ("Cartridge case", 0.1, f'<rect x="-36" y="-12" width="66" height="24" rx="3" {S}/><path d="M 30 -14 L 36 -14 L 36 14 L 30 14" {S}/>'),
    "firearm": ("Firearm", 0.3, f'<path d="M -44 -12 L 36 -12 L 36 2 L -10 2 L -18 30 L -34 30 L -30 2 L -44 2 Z" {S}/>'),
    "knife": ("Knife", 0.3, f'<path d="M -44 -6 L -6 -6 L -6 6 L -44 6 Z" {S}/><path d="M -6 -9 L 30 -9 Q 44 -4 44 0 L -6 6" {S}/>'),
    "shoe_mark": ("Footwear impression", 0.3, f'<path d="M -40 -12 Q -46 0 -40 12 Q -20 16 0 10 Q 30 14 42 6 Q 46 0 42 -6 Q 30 -14 0 -10 Q -20 -16 -40 -12 Z" {S}/>'),
    "tire_mark": ("Tire mark", 3.0, f'<path d="M -46 -12 L 46 -12 M -46 12 L 46 12" {S} stroke-dasharray="10 6"/>'),
    "impact": ("Point of impact", 1.0, f'<path d="M 0 -44 L 9 -12 L 40 -18 L 16 4 L 34 30 L 2 16 L -18 40 L -14 8 L -44 0 L -14 -10 Z" {S}/>'),
    "camera": ("Camera position", 0.5, f'<rect x="-30" y="-18" width="44" height="36" rx="4" {S}/><path d="M 14 -8 L 40 -22 L 40 22 L 14 8" {S}/>'),
    "tree": ("Tree", 4.0, f'<circle cx="0" cy="0" r="42" {S} stroke-dasharray="14 6"/><circle cx="0" cy="0" r="6" {F}/>'),
    "pole": ("Pole or post", 0.3, f'<circle cx="0" cy="0" r="20" {S}/><path d="M -14 -14 L 14 14 M -14 14 L 14 -14" {S}/>'),
}

index = []
for sid, (name, size, body) in symbols.items():
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="-50 -50 100 100" width="100" height="100">'
           f'<title>{name}</title>{body}</svg>\n')
    (out / f"{sid}.svg").write_text(svg, encoding="utf-8", newline="\n")
    index.append({"id": sid, "name": name, "size": size})

(out / "index.json").write_text(json.dumps(index, indent=2) + "\n", encoding="utf-8", newline="\n")
(out / "README.md").write_text(
    "# Diagram symbols\n\n"
    "Original symbols drawn for Locus (nothing copied from other products). Each file is a 100 × 100 "
    "SVG centred on the origin, drawn in `currentColor`, seen from above; `index.json` gives each "
    "symbol's name and its default size on the ground in metres.\n\n"
    "`make_symbols.py` writes all of them: change a drawing there and run "
    "`py assets/symbols/make_symbols.py`. The editor (app/src/diagram2d/symbols.ts) and printing "
    "(crates/locus-report/src/diagram.rs) both read these files; a test checks the two lists match.\n",
    encoding="utf-8", newline="\n")
print(len(index), "symbols")
