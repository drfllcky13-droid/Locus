// Registration report. Every value arrives formatted in data.json (see registration.rs);
// this template only lays it out.
#let d = json("data.json")

#set document(title: d.title)
#set text(font: "Go", size: 9pt, hyphenate: false, lang: "en")
#set par(justify: false)
#set page(
  paper: "a4",
  margin: (x: 16mm, top: 20mm, bottom: 18mm),
  header: context [
    #set text(size: 8pt, fill: luma(90))
    #d.header #h(1fr) Registration report
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

#let grid-table(columns, head, rows) = table(
  columns: columns,
  stroke: 0.4pt + luma(170),
  inset: 4pt,
  table.header(..head.map(h => text(weight: "bold")[#h])),
  ..rows.flatten().map(c => [#c]),
)

= #d.title

#pairs(d.details)

== Summary
#pairs(d.summary)

#if d.warnings.len() > 0 [
  #block(stroke: 0.8pt + rgb("#b3261e"), inset: 8pt, width: 100%)[
    #text(weight: "bold")[Needs attention]
    #for w in d.warnings [
      - #w
    ]
  ]
]

== Settings
#pairs(d.settings)

== Scan poses
Position of each scan's origin in the project frame (m) and heading (degrees from +x, anticlockwise seen from above). A scan is verified when links that are neither flagged nor based on shape alone tie it to the reference.

#grid-table(
  (1fr, auto, auto, auto, auto, auto, auto),
  ("Scan", "Points", "x", "y", "z", "Heading", "Verified"),
  d.scans,
)

== Links
Residuals are distances between paired points after the adjustment. χ²/dof is compared with its 99.9 % limit; a link above the limit is inconsistent with the others and is set aside. Overlap is the share of one scan's points that paired with the other's (cloud links).

#set text(size: 8pt)
#grid-table(
  (auto, auto, 1fr, 1.2fr, auto, auto, auto, auto, auto, auto),
  ("#", "Kind", "Scans", "Result", "Pairs", "RMS", "Largest", "χ²/dof", "Limit", "Overlap"),
  d.links,
)
