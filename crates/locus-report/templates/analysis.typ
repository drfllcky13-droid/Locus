// Analysis report (every analysis tool). All values arrive formatted in data.json (see
// analysis.rs and each tool's module); this template only lays them out.
#let d = json("data.json")

#set document(title: d.title)
#set text(font: "Go", size: 9pt, hyphenate: false, lang: "en")
#set par(justify: false)
#set page(
  paper: "a4",
  margin: (x: 16mm, top: 20mm, bottom: 18mm),
  header: context [
    #set text(size: 8pt, fill: luma(90))
    #d.header
  ],
  footer: context [
    #set text(size: 8pt, fill: luma(90))
    #h(1fr) Page #counter(page).display() of #counter(page).final().first()
  ],
)
#show heading: set text(size: 11pt)
#show heading.where(level: 1): set text(size: 15pt)

#let pairs(rows) = table(
  columns: (auto, 1fr),
  stroke: none,
  inset: (x: 0pt, y: 3pt),
  column-gutter: 12pt,
  ..rows.map(r => (text(fill: luma(70))[#r.at(0)], [#r.at(1)])).flatten(),
)

#let widths(ws) = ws.map(w => if w == "auto" { auto } else if w.ends-with("fr") {
  float(w.slice(0, -2)) * 1fr
} else { float(w.slice(0, -2)) * 1mm })

#let grid-table(b) = table(
  columns: widths(b.widths),
  stroke: 0.4pt + luma(170),
  inset: 4pt,
  table.header(..b.head.map(h => text(weight: "bold")[#h])),
  ..b.rows.flatten().map(c => [#c]),
)

#let figure-box(f) = {
  let mm = 1mm
  block(breakable: false, width: 100%)[
    #box(width: f.width * mm, height: f.height * mm, stroke: 0.4pt + luma(170), clip: true)[
      #for a in f.areas {
        place(top + left, polygon(fill: rgb(a.fill), stroke: none,
          ..a.points.map(p => (p.at(0) * mm, p.at(1) * mm))))
      }
      #for l in f.lines {
        place(top + left, line(
          start: (l.at(0) * mm, l.at(1) * mm),
          end: (l.at(2) * mm, l.at(3) * mm),
          stroke: (paint: black, thickness: l.at(4) * mm, dash: if l.at(5) > 0 { "dashed" } else { none }),
        ))
      }
      #for p in f.dots {
        place(top + left, dx: (p.at(0) - p.at(2)) * mm, dy: (p.at(1) - p.at(2)) * mm,
          circle(radius: p.at(2) * mm, fill: black))
      }
      #for t in f.labels {
        place(top + left, dx: t.x * mm, dy: t.y * mm, text(size: 7pt)[#t.text])
      }
    ]
    #v(2pt)
    #text(size: 8pt, fill: luma(70))[#f.caption]
  ]
}

= #d.title

#pairs(d.details)

#if d.warnings.len() > 0 [
  #block(stroke: 0.8pt + rgb("#b3261e"), inset: 8pt, width: 100%)[
    #text(weight: "bold")[Needs attention]
    #for w in d.warnings [
      - #w
    ]
  ]
]

#for s in d.sections [
  == #s.heading
  #for b in s.blocks {
    if b.kind == "text" [#par[#b.text]]
    else if b.kind == "pairs" { pairs(b.rows) }
    else if b.kind == "table" { grid-table(b) }
    else if b.kind == "list" [
      #for i in b.items [
        - #i
      ]
    ]
    else if b.kind == "figure" { figure-box(b) }
  }
]
