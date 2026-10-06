import SwiftUI

// MARK: - Ruler

/// A ruler projected onto the canvas: where its corners landed this frame.
private struct RulerGeometry {
    let rect: RulerRect
    /// Bounding box of the two anchors.
    let box: CGRect
    /// Where the measurement started and ended, which is not the same as the box's
    /// top-left and bottom-right.
    let from: CGPoint
    let to: CGPoint
    let tint: Color
    let isDraft: Bool
}

extension ChartPlot {
    /// Measuring rectangles, plus the one being drawn right now.
    ///
    /// Call inside the series clip. The bull/bear colours are passed in rather than read
    /// off `style`: they are a per-chart setting, and which way the measured move went is
    /// the whole point of the tool, so the rectangle has to agree with the candles under it.
    ///
    /// Boxes, guides and handles are drawn first and every read-out card after them, so a
    /// card is never buried under a neighbouring box.
    func drawRulers(
        _ context: inout GraphicsContext,
        overlay: RulerOverlayState,
        points: [KlineData],
        bullish: Color,
        bearish: Color
    ) {
        guard !points.isEmpty, !overlay.isEmpty else { return }
        let slot = slotWidth(forCount: points.count)

        var rects = overlay.rects.map { (rect: $0, isDraft: false) }
        if let draft = overlay.draft {
            rects.append((RulerRect(start: draft.start, end: draft.end), true))
        }
        let geometry = rects.map {
            rulerGeometry(
                $0.rect, points: points, slot: slot, bullish: bullish, bearish: bearish, isDraft: $0.isDraft)
        }

        for item in geometry {
            let isEmphasised =
                !item.isDraft && (item.rect.id == overlay.selectedID || item.rect.id == overlay.hover?.id)
            drawRulerBox(&context, item: item, isEmphasised: isEmphasised)
            drawRulerGuides(&context, item: item)
            if isEmphasised {
                let hoveredCorner: RulerCorner? = {
                    guard let hover = overlay.hover, hover.id == item.rect.id, case .corner(let corner) = hover.part
                    else { return nil }
                    return corner
                }()
                drawRulerHandles(&context, item: item, hovered: hoveredCorner)
            }
        }

        for item in geometry {
            drawRulerCard(&context, item: item, points: points)
        }
    }

    /// Price tags in the right-hand gutter for the end prices of the rectangle being drawn,
    /// the one under the pointer or selected, and the newest one otherwise. Not every
    /// ruler: a handful of finished measurements would bury the axis.
    ///
    /// Call outside the series clip — the gutter is outside the plot by design.
    func drawRulerPriceTags(
        _ context: inout GraphicsContext,
        overlay: RulerOverlayState,
        bullish: Color,
        bearish: Color
    ) {
        guard !overlay.isEmpty else { return }

        var tagged: [(rect: RulerRect, tint: Color)] = []
        for rect in overlay.rects {
            let isEmphasised = rect.id == overlay.selectedID || rect.id == overlay.hover?.id
            let isNewest = rect.id == overlay.rects.last?.id && overlay.selectedID == nil && overlay.hover == nil
            if isEmphasised || isNewest {
                tagged.append((rect, rect.isUpward ? bullish : bearish))
            }
        }
        if let draft = overlay.draft {
            let rect = RulerRect(start: draft.start, end: draft.end)
            tagged.append((rect, rect.isUpward ? bullish : bearish))
        }

        for item in tagged {
            for anchor in [item.rect.start, item.rect.end] {
                drawRulerPriceTag(&context, price: anchor.price, tint: item.tint)
            }
        }
    }

    // MARK: Geometry

    private func rulerGeometry(
        _ rect: RulerRect,
        points: [KlineData],
        slot: CGFloat,
        bullish: Color,
        bearish: Color,
        isDraft: Bool
    ) -> RulerGeometry {
        let from = position(of: rect.start, points: points, slotWidth: slot)
        let to = position(of: rect.end, points: points, slotWidth: slot)
        let box = CGRect(
            x: min(from.x, to.x),
            y: min(from.y, to.y),
            width: abs(to.x - from.x),
            height: abs(to.y - from.y)
        )
        return RulerGeometry(
            rect: rect, box: box, from: from, to: to,
            tint: rect.isUpward ? bullish : bearish, isDraft: isDraft)
    }

    // MARK: Box

    private func drawRulerBox(
        _ context: inout GraphicsContext,
        item: RulerGeometry,
        isEmphasised: Bool
    ) {
        let box = item.box
        let radius = min(style.rulerCornerRadius, box.width / 2, box.height / 2)
        let path = Path(roundedRect: box, cornerRadius: max(0, radius))

        // Strongest at the edge the measurement ended on, so the box points the way the
        // move went. A flat box has no axis to fade along.
        if box.height >= 1 {
            let gradient = Gradient(colors: [
                item.tint.opacity(style.rulerFillFadeOpacity),
                item.tint.opacity(style.rulerFillOpacity),
            ])
            context.fill(
                path,
                with: .linearGradient(
                    gradient,
                    startPoint: CGPoint(x: box.midX, y: item.from.y),
                    endPoint: CGPoint(x: box.midX, y: item.to.y)
                )
            )
        } else {
            context.fill(path, with: .color(item.tint.opacity(style.rulerFillOpacity)))
        }

        context.stroke(
            path,
            with: .color(item.tint.opacity(isEmphasised ? style.rulerEmphasisBorderOpacity : style.rulerBorderOpacity)),
            style: StrokeStyle(
                lineWidth: isEmphasised ? style.rulerEmphasisBorderWidth : style.rulerBorderWidth,
                dash: item.isDraft ? style.rulerDashPattern : []
            )
        )
    }

    // MARK: Guides

    /// A centred arrow along each axis — start price to end price, start time to end
    /// time — and a dot on the corner the measurement began at. Left out on an axis the
    /// box is too short on for an arrow to read.
    private func drawRulerGuides(
        _ context: inout GraphicsContext,
        item: RulerGeometry
    ) {
        let color = item.tint.opacity(style.rulerGuideOpacity)
        let minimum: CGFloat = 24

        if abs(item.to.y - item.from.y) >= minimum {
            drawRulerArrow(
                &context,
                from: CGPoint(x: item.box.midX, y: item.from.y),
                to: CGPoint(x: item.box.midX, y: item.to.y),
                color: color
            )
        }
        if abs(item.to.x - item.from.x) >= minimum {
            drawRulerArrow(
                &context,
                from: CGPoint(x: item.from.x, y: item.box.midY),
                to: CGPoint(x: item.to.x, y: item.box.midY),
                color: color
            )
        }

        let dot: CGFloat = 2.5
        context.fill(
            Path(ellipseIn: CGRect(x: item.from.x - dot, y: item.from.y - dot, width: dot * 2, height: dot * 2)),
            with: .color(color)
        )
    }

    private func drawRulerArrow(
        _ context: inout GraphicsContext,
        from: CGPoint,
        to: CGPoint,
        color: Color
    ) {
        let dx = to.x - from.x
        let dy = to.y - from.y
        let length = hypot(dx, dy)
        guard length > 0 else { return }

        let head = style.rulerArrowheadSize
        let ux = dx / length
        let uy = dy / length
        // Perpendicular to the shaft.
        let nx = -uy
        let ny = ux
        let base = CGPoint(x: to.x - ux * head, y: to.y - uy * head)

        var shaft = Path()
        shaft.move(to: from)
        shaft.addLine(to: base)
        context.stroke(shaft, with: .color(color), lineWidth: style.rulerGuideWidth)

        var tip = Path()
        tip.move(to: to)
        tip.addLine(to: CGPoint(x: base.x + nx * head * 0.6, y: base.y + ny * head * 0.6))
        tip.addLine(to: CGPoint(x: base.x - nx * head * 0.6, y: base.y - ny * head * 0.6))
        tip.closeSubpath()
        context.fill(tip, with: .color(color))
    }

    // MARK: Handles

    private func drawRulerHandles(
        _ context: inout GraphicsContext,
        item: RulerGeometry,
        hovered: RulerCorner?
    ) {
        for corner in RulerCorner.allCases {
            let center = CGPoint(
                x: corner.isLeft ? item.box.minX : item.box.maxX,
                y: corner.isTop ? item.box.minY : item.box.maxY
            )
            let isHovered = corner == hovered
            let radius = style.rulerHandleRadius + (isHovered ? 1.5 : 0)
            let circle = Path(
                ellipseIn: CGRect(
                    x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2)
            )
            context.fill(circle, with: .color(isHovered ? item.tint : .white))
            context.stroke(circle, with: .color(isHovered ? .white : item.tint), lineWidth: 1.5)
        }
    }

    // MARK: Read-out card

    /// Percent on top in the move's colour, the price change under it, and how much chart
    /// it covers in a muted third line.
    private func drawRulerCard(
        _ context: inout GraphicsContext,
        item: RulerGeometry,
        points: [KlineData]
    ) {
        let readout = RulerReadout(rect: item.rect, points: points)

        let glyph = context.resolve(
            Text(readout.isUpward ? "▲" : "▼")
                .font(.system(size: 8, weight: .bold))
                .foregroundStyle(item.tint)
        )
        let percent = context.resolve(
            Text(readout.percentText)
                .font(.system(size: 13, weight: .bold, design: .rounded))
                .monospacedDigit()
                .foregroundStyle(item.tint)
        )
        let price = context.resolve(
            Text(readout.priceText(decimalPlaces: yAxisDecimalPlaces, scale: scale))
                .font(.caption2)
                .bold()
                .monospacedDigit()
                .foregroundStyle(.white.opacity(0.9))
        )
        let coverage = context.resolve(
            Text(readout.coverageText)
                .font(.caption2)
                .foregroundStyle(.white.opacity(0.55))
        )

        let measureBox = CGSize(width: 240, height: 40)
        let glyphSize = glyph.measure(in: measureBox)
        let percentSize = percent.measure(in: measureBox)
        let priceSize = price.measure(in: measureBox)
        let coverageSize = coverage.measure(in: measureBox)

        let glyphGap: CGFloat = 4
        let headlineWidth = glyphSize.width + glyphGap + percentSize.width
        let headlineHeight = max(glyphSize.height, percentSize.height)

        let padding = style.rulerCardPadding
        let spacing = style.rulerCardRowSpacing
        let contentWidth = max(headlineWidth, priceSize.width, coverageSize.width)
        let size = CGSize(
            width: contentWidth + padding.width * 2,
            height: headlineHeight + priceSize.height + coverageSize.height + spacing * 2 + padding.height * 2
        )

        let origin = rulerCardOrigin(size: size, box: item.box, isUpward: item.rect.isUpward)
        let card = CGRect(origin: origin, size: size)
        let cardPath = Path(roundedRect: card, cornerRadius: style.rulerCardCornerRadius, style: .continuous)

        context.drawLayer { layer in
            layer.addFilter(.shadow(color: .black.opacity(0.35), radius: 6, x: 0, y: 2))
            layer.fill(cardPath, with: .color(style.rulerCardColor))
        }
        context.stroke(cardPath, with: .color(item.tint.opacity(style.rulerCardBorderOpacity)), lineWidth: 1)

        var y = card.minY + padding.height
        let headlineX = card.midX - headlineWidth / 2
        let headlineY = y + headlineHeight / 2
        context.draw(glyph, at: CGPoint(x: headlineX, y: headlineY), anchor: .leading)
        context.draw(
            percent, at: CGPoint(x: headlineX + glyphSize.width + glyphGap, y: headlineY), anchor: .leading)
        y += headlineHeight + spacing

        context.draw(price, at: CGPoint(x: card.midX, y: y + priceSize.height / 2))
        y += priceSize.height + spacing

        context.draw(coverage, at: CGPoint(x: card.midX, y: y + coverageSize.height / 2))
    }

    /// Outside the edge the measurement ended on — above when measuring up, below when
    /// measuring down — so the card doesn't cover the candles being read. When that side
    /// has no room it takes the opposite side, and failing that sits inside the box.
    private func rulerCardOrigin(size: CGSize, box: CGRect, isUpward: Bool) -> CGPoint {
        let gap = style.rulerCardGap
        let above = box.minY - size.height - gap
        let below = box.maxY + gap
        let fitsAbove = above >= plotRect.minY
        let fitsBelow = below + size.height <= plotRect.maxY

        let sides = isUpward ? [(above, fitsAbove), (below, fitsBelow)] : [(below, fitsBelow), (above, fitsAbove)]
        // Neither side fits: tuck it inside the box next to the end edge.
        var y =
            sides.first(where: { $0.1 })?.0
            ?? (isUpward ? box.minY + gap : box.maxY - size.height - gap)
        y = y.clamped(to: plotRect.minY...max(plotRect.minY, plotRect.maxY - size.height))

        let x = (box.midX - size.width / 2)
            .clamped(to: plotRect.minX...max(plotRect.minX, plotRect.maxX - size.width))
        return CGPoint(x: x, y: y)
    }

    // MARK: Gutter tags

    /// Same pill as the current-price marker, in the measurement's colour.
    private func drawRulerPriceTag(_ context: inout GraphicsContext, price: Double, tint: Color) {
        let y = self.y(for: price)
        // A Y-zoom since the rectangle was drawn can push an end off the plot; clamping
        // the tag to the edge would misreport where it is.
        guard y >= plotRect.minY, y <= plotRect.maxY else { return }

        let label = context.resolve(
            Text(PriceFormatter.format(price, decimalPlaces: yAxisDecimalPlaces, scale: scale))
                .font(.caption2)
                .bold()
                .foregroundStyle(.white)
        )
        let textSize = label.measure(in: CGSize(width: 100, height: 20))
        let height: CGFloat = 18
        guard let originY = Self.overlayOriginY(centeredAt: y, in: plotRect, labelHeight: height) else { return }

        let rect = CGRect(x: plotRect.maxX + 4, y: originY, width: textSize.width + 10, height: height)
        context.fill(Path(roundedRect: rect, cornerRadius: 3), with: .color(tint))
        context.draw(label, at: CGPoint(x: rect.midX, y: rect.midY))
    }
}
