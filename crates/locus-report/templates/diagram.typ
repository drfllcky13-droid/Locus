// A diagram sheet at scale. Every position and size arrives in page millimetres from
// diagram.rs (origin top left, y down); this template only places them.
#let d = json("data.json")
#let mm = 1mm

#set document(title: d.title)
#set page(width: d.page.at(0) * mm, height: d.page.at(1) * mm, margin: 0pt)
#set text(font: "Go", size: 8pt, hyphenate: false, lang: "en")

#let at(x, y, body) = place(top + left, dx: x * mm, dy: y * mm, body)
#let ink = black

// Frame.
#let f = d.frame
#at(f.at(0), f.at(1), rect(width: (f.at(2) - f.at(0)) * mm, height: (f.at(3) - f.at(1)) * mm, stroke: 0.5pt + ink))

// Geometry.
#for l in d.lines {
  place(top + left, line(start: (l.at(0) * mm, l.at(1) * mm), end: (l.at(2) * mm, l.at(3) * mm), stroke: l.at(4) * mm + ink))
}
#for p in d.paths {
  place(top + left, curve(
    stroke: 0.35 * mm + ink,
    curve.move((p.at(0).at(0) * mm, p.at(0).at(1) * mm)),
    ..p.slice(1).map(q => curve.line((q.at(0) * mm, q.at(1) * mm))),
  ))
}

// Symbols, centred on their position.
#for s in d.symbols {
  let (file, x, y, w, r) = s
  at(x - w / 2, y - w / 2, rotate(r * 1deg, image(file, width: w * mm, height: w * mm)))
}

// Reference and measured points.
#for p in d.points {
  let (x, y, label, measured) = p
  at(x - 0.8, y - 0.8, circle(radius: 0.8 * mm, stroke: 0.25 * mm + ink))
  if measured { at(x - 1.6, y - 1.6, circle(radius: 1.6 * mm, stroke: (paint: ink, thickness: 0.2 * mm, dash: "dotted"))) }
  if label != "" { at(x + 1.5, y - 3.5, text(size: 2.5 * mm)[#label]) }
}

// Evidence markers: numbered triangles.
#for m in d.markers {
  let (x, y, n) = m
  at(x - 2.5, y - 3, polygon(fill: rgb("#f2c94c"), stroke: 0.2 * mm + ink, (0mm, 5mm), (2.5mm, 0mm), (5mm, 5mm)))
  at(x - 2.5, y - 1.2, box(width: 5 * mm, align(center, text(size: 2 * mm, weight: "bold")[#n])))
}

// Text.
#for t in d.texts {
  let body = text(size: t.size * mm)[#t.text]
  if t.centred {
    // Centred on the point, sitting just above it, turned with the line about that point:
    // a wide, zero-height box whose top centre is the point.
    at(t.x - 100, t.y, rotate(t.rotation * 1deg, origin: top + center, reflow: false,
      box(width: 200 * mm, height: 0pt, place(bottom + center, dy: -0.8 * mm, body))))
  } else {
    at(t.x, t.y, rotate(t.rotation * 1deg, origin: top + left, reflow: false, body))
  }
}

// North arrows.
#for n in d.norths {
  let (x, y, r) = n
  at(x - 4, y - 7, rotate(r * 1deg, box(width: 8 * mm, height: 14 * mm)[
    #place(top + left, polygon(fill: ink, (4mm, 0mm), (7mm, 9mm), (4mm, 7mm), (1mm, 9mm)))
    #place(top + left, dy: 10 * mm, box(width: 8 * mm, align(center, text(size: 3 * mm, weight: "bold")[N])))
  ]))
}

// Scale bars: half filled, exact length.
#for b in d.scalebars {
  let (x, y, len, label) = b
  at(x, y - 1.5, rect(width: len / 2 * mm, height: 1.5 * mm, fill: ink, stroke: none))
  at(x, y - 1.5, rect(width: len * mm, height: 1.5 * mm, stroke: 0.25 * mm + ink))
  at(x, y + 0.8, text(size: 2.2 * mm)[0])
  at(x + len - 12, y + 0.8, box(width: 12 * mm, align(right, text(size: 2.2 * mm)[#label])))
}

// Legends.
#for g in d.legends {
  let (x, y, rows) = g
  at(x, y, block(stroke: 0.3 * mm + ink, inset: 2 * mm, fill: white, width: 55 * mm)[
    #text(weight: "bold")[Legend]
    #for r in rows {
      let (label, glyph) = r
      let icon = if glyph == "marker" {
        polygon(fill: rgb("#f2c94c"), stroke: 0.2 * mm + ink, (0mm, 3.5mm), (1.75mm, 0mm), (3.5mm, 3.5mm))
      } else if glyph == "point" {
        circle(radius: 0.8 * mm, stroke: 0.25 * mm + ink)
      } else if glyph == "measured" {
        circle(radius: 1.4 * mm, stroke: (paint: ink, thickness: 0.2 * mm, dash: "dotted"))
      } else {
        image(glyph, width: 4 * mm, height: 4 * mm)
      }
      block(above: 1.5 * mm, grid(columns: (6 * mm, 1fr), align: horizon, icon, text(size: 2.5 * mm)[#label]))
    }
  ])
}

// Calibration bar: exactly 100 mm, ticks every 10 mm, in the title block, so a printout
// can be checked with a ruler.
#let cal-x = (f.at(0) + f.at(2)) / 2 - 50
#let cal-y = f.at(3) + 24
#at(cal-x, cal-y, rect(width: 100 * mm, height: 2 * mm, stroke: 0.25 * mm + ink))
#for i in range(11) {
  place(top + left, line(start: ((cal-x + i * 10) * mm, (cal-y - 1) * mm), end: ((cal-x + i * 10) * mm, (cal-y + 2) * mm), stroke: 0.2 * mm + ink))
}
#at(cal-x, cal-y + 3, box(width: 100 * mm, align(center, text(size: 6.5pt)[Calibration bar: 100 mm. Measure it to check the print scale.])))

// Title block.
#at(f.at(0), f.at(3) + 3, block(width: (f.at(2) - f.at(0)) * mm)[
  #grid(
    columns: (1fr, auto),
    column-gutter: 6 * mm,
    [
      #text(size: 12pt, weight: "bold")[#d.title] \
      #for r in d.details [#text(fill: luma(60))[#r.at(0):] #r.at(1) \ ]
    ],
    align(right)[
      #text(size: 14pt, weight: "bold")[Scale #d.scale_label] \
      #text(size: 7pt)[Print at actual size (100 %). \ Do not scale to fit the page.]
    ],
  )
])
