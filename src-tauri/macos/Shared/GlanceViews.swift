import SwiftUI

/// The capsule meter (the popup's `Meter`): a faint track and a flat fill in the pace color. Any
/// non-zero fill is at least as wide as the bar is tall, so 1-2% never disappears.
struct GlanceMeter: View {
    let fraction: Double
    let severity: GlanceSeverity
    var onDark = false
    var height: CGFloat = 5

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let clamped = min(max(fraction, 0), 1)
            let fill = clamped > 0 ? max(width * clamped, height) : 0
            ZStack(alignment: .leading) {
                Capsule().fill(onDark ? Color.white.opacity(0.16) : Color.primary.opacity(0.12))
                if fill > 0 {
                    Capsule()
                        .fill(GlancePalette.fill(severity, onDark: onDark))
                        .frame(width: min(fill, width))
                }
            }
        }
        .frame(height: height)
    }
}

/// A round gauge: a faint track and an arc in the pace color starting at twelve o'clock, with
/// whatever `center` draws inside. A metric without a limit draws the track alone.
struct GlanceRing<Center: View>: View {
    let fraction: Double?
    let severity: GlanceSeverity
    var onDark = false
    var lineWidth: CGFloat = 4
    @ViewBuilder var center: () -> Center

    var body: some View {
        ZStack {
            Circle()
                .stroke(onDark ? Color.white.opacity(0.16) : Color.primary.opacity(0.12), lineWidth: lineWidth)
            if let fraction, fraction > 0 {
                Circle()
                    .trim(from: 0, to: min(max(fraction, 0.03), 1))
                    .stroke(
                        GlancePalette.fill(severity, onDark: onDark),
                        style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)
                    )
                    .rotationEffect(.degrees(-90))
            }
            center()
        }
        .padding(lineWidth / 2)
    }
}

/// When the metric comes back: a countdown against `now`, the reset moment itself, or the metric's
/// own detail text.
struct GlanceResetText: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date

    var body: some View {
        Text(text)
            .monospacedDigit()
            .lineLimit(1)
    }

    private var text: String {
        if let resetsAt = metric.resetsAt {
            if resetsAt <= now { return labels.resetting }
            return "\(labels.resetsIn) \(GlanceFormat.countdown(to: resetsAt, from: now, units: labels.units))"
        }
        return metric.detail ?? ""
    }
}

/// One metric: its label and headline over the meter, and the reset countdown beneath.
struct GlanceMetricRow: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date
    var onDark = false
    var compact = false
    var showsReset = true

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 2 : 3) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(metric.label)
                    .font(.system(size: compact ? 10.5 : 11.5, weight: .medium))
                    .foregroundStyle(onDark ? Color.white.opacity(0.72) : Color.secondary)
                    .lineLimit(1)
                Spacer(minLength: 4)
                Text(metric.headline)
                    .font(.system(size: compact ? 11 : 12.5, weight: .semibold))
                    .foregroundStyle(GlancePalette.text(metric.severity, onDark: onDark))
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            if let fraction = metric.fraction {
                GlanceMeter(fraction: fraction, severity: metric.severity, onDark: onDark, height: compact ? 4 : 5)
            }
            if showsReset {
                GlanceResetText(metric: metric, labels: labels, now: now)
                    .font(.system(size: compact ? 9.5 : 10.5))
                    .foregroundStyle(onDark ? Color.white.opacity(0.55) : Color.secondary.opacity(0.9))
            }
        }
    }
}

/// A provider's mark in its brand color beside its name and plan, with the account's email under.
struct GlanceProviderHeader: View {
    let provider: GlanceProvider
    var shows: GlanceShows = .all
    var onDark = false
    var size: CGFloat = 13

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 5) {
                ProviderMark(mark: provider.mark)
                    .foregroundStyle(onDark ? provider.tint : markColor)
                    .frame(width: size, height: size)
                Text(provider.name)
                    .font(.system(size: size - 1, weight: .semibold))
                    .foregroundStyle(onDark ? Color.white : Color.primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if shows.plan, let plan = provider.plan {
                    GlancePlanBadge(text: plan, onDark: onDark, size: size)
                }
            }
            if shows.account, let account = provider.account {
                Text(account)
                    .font(.system(size: max(size - 3.5, 8.5)))
                    .foregroundStyle(onDark ? Color.white.opacity(0.55) : Color.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.leading, size + 5)
            }
        }
    }

    private var markColor: Color {
        provider.color.uppercased() == "#FFFFFF" ? .primary : provider.tint
    }
}

/// The plan (`Pro`, `Max 5x`) in a small rounded tag.
struct GlancePlanBadge: View {
    let text: String
    var onDark = false
    var size: CGFloat = 13

    var body: some View {
        Text(text)
            .font(.system(size: max(size - 4, 8), weight: .semibold))
            .foregroundStyle(onDark ? Color.white.opacity(0.7) : Color.secondary)
            .lineLimit(1)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(
                RoundedRectangle(cornerRadius: 3, style: .continuous)
                    .fill(onDark ? Color.white.opacity(0.12) : Color.primary.opacity(0.08))
            )
            .fixedSize()
    }
}

/// Why an account shows no readings: signed out, session expired, no data yet.
struct GlanceNoticeRow: View {
    let text: String
    var onDark = false
    var size: CGFloat = 10.5

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: size - 1))
            Text(text)
                .font(.system(size: size))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(onDark ? Color(red: 1.0, green: 0.62, blue: 0.04) : Color.orange)
    }
}
