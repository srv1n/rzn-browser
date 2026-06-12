import React from "react";
import {
  AbsoluteFill,
  Easing,
  Sequence,
  interpolate,
  spring,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

const palette = {
  ink: "#06131d",
  inkSoft: "#0b1b29",
  slate: "#163049",
  cyan: "#6ce7ff",
  teal: "#2ed3b7",
  lime: "#b8ff6a",
  gold: "#ffd36e",
  coral: "#ff8c6b",
  white: "#f6fbff",
  mist: "rgba(246, 251, 255, 0.74)",
  line: "rgba(108, 231, 255, 0.22)",
};

const fpsFromSeconds = (seconds: number, fps: number) => seconds * fps;

const entrance = (frame: number, fps: number, delaySeconds = 0) =>
  spring({
    fps,
    frame: Math.max(0, frame - fpsFromSeconds(delaySeconds, fps)),
    config: {
      damping: 18,
      stiffness: 140,
      mass: 0.8,
    },
  });

const fadeWindow = (
  frame: number,
  fps: number,
  startSeconds: number,
  endSeconds: number,
) => {
  const start = fpsFromSeconds(startSeconds, fps);
  const end = fpsFromSeconds(endSeconds, fps);

  if (frame <= start || frame >= end) {
    return 0;
  }

  const fadeIn = interpolate(frame, [start, start + fps * 0.5], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const fadeOut = interpolate(frame, [end - fps * 0.5, end], [1, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.7, 0, 0.84, 0),
  });

  return Math.min(fadeIn, fadeOut);
};

const drift = (frame: number, amplitude: number, speed = 0.06) =>
  Math.sin(frame * speed) * amplitude;

const Background: React.FC = () => {
  const frame = useCurrentFrame();

  return (
    <AbsoluteFill
      style={{
        background: `radial-gradient(circle at 15% 20%, rgba(46, 211, 183, 0.14), transparent 32%),
          radial-gradient(circle at 85% 18%, rgba(255, 140, 107, 0.18), transparent 28%),
          radial-gradient(circle at 50% 85%, rgba(108, 231, 255, 0.12), transparent 36%),
          linear-gradient(145deg, ${palette.ink} 0%, ${palette.inkSoft} 45%, #09101a 100%)`,
      }}
    >
      {Array.from({ length: 18 }).map((_, index) => {
        const size = 140 + (index % 4) * 90;
        const top = 6 + (index % 6) * 15;
        const left = 2 + ((index * 11) % 90);
        const opacity = 0.06 + (index % 3) * 0.025;
        const x = drift(frame + index * 5, 24 + index * 1.6, 0.012 + index * 0.001);
        const y = drift(frame + index * 7, 18 + index, 0.015 + index * 0.0012);

        return (
          <div
            key={index}
            style={{
              position: "absolute",
              top: `${top}%`,
              left: `${left}%`,
              width: size,
              height: size,
              borderRadius: "50%",
              background:
                index % 2 === 0
                  ? "radial-gradient(circle, rgba(108, 231, 255, 0.18), transparent 72%)"
                  : "radial-gradient(circle, rgba(255, 211, 110, 0.14), transparent 72%)",
              opacity,
              transform: `translate(${x}px, ${y}px)`,
              filter: "blur(18px)",
            }}
          />
        );
      })}
      <div
        style={{
          position: "absolute",
          inset: 48,
          borderRadius: 36,
          border: `1px solid ${palette.line}`,
          opacity: 0.55,
        }}
      />
    </AbsoluteFill>
  );
};

const SectionHeading: React.FC<{
  eyebrow: string;
  title: string;
  subtitle: string;
  align?: "left" | "center";
}> = ({ eyebrow, title, subtitle, align = "left" }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const reveal = entrance(frame, fps, 0.1);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: align === "center" ? "center" : "flex-start",
        textAlign: align,
        gap: 20,
        transform: `translateY(${(1 - reveal) * 42}px)`,
        opacity: reveal,
      }}
    >
      <div
        style={{
          padding: "12px 20px",
          borderRadius: 999,
          border: `1px solid ${palette.line}`,
          color: palette.cyan,
          fontSize: 26,
          fontWeight: 700,
          letterSpacing: 2.6,
          textTransform: "uppercase",
          background: "rgba(6, 19, 29, 0.55)",
        }}
      >
        {eyebrow}
      </div>
      <div
        style={{
          fontSize: 86,
          lineHeight: 0.96,
          color: palette.white,
          fontWeight: 800,
          maxWidth: 1120,
          letterSpacing: -2.8,
        }}
      >
        {title}
      </div>
      <div
        style={{
          fontSize: 34,
          lineHeight: 1.34,
          color: palette.mist,
          maxWidth: 1040,
          fontWeight: 500,
        }}
      >
        {subtitle}
      </div>
    </div>
  );
};

const GlassCard: React.FC<{
  title: string;
  body: string;
  accent: string;
  width?: number;
  height?: number;
  style?: React.CSSProperties;
}> = ({ title, body, accent, width = 320, height = 220, style }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const reveal = entrance(frame, fps);

  return (
    <div
      style={{
        width,
        height,
        borderRadius: 28,
        padding: 28,
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        background:
          "linear-gradient(180deg, rgba(12, 32, 49, 0.82), rgba(8, 21, 33, 0.78))",
        border: `1px solid ${palette.line}`,
        boxShadow: `0 22px 80px rgba(0, 0, 0, 0.26), inset 0 0 0 1px ${accent}26`,
        transform: `translateY(${(1 - reveal) * 36}px) scale(${0.96 + reveal * 0.04})`,
        opacity: reveal,
        ...style,
      }}
    >
      <div
        style={{
          width: 58,
          height: 58,
          borderRadius: 18,
          background: `linear-gradient(135deg, ${accent}, rgba(255,255,255,0.14))`,
          boxShadow: `0 0 34px ${accent}44`,
        }}
      />
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <div
          style={{
            fontSize: 34,
            color: palette.white,
            fontWeight: 760,
            lineHeight: 1.05,
          }}
        >
          {title}
        </div>
        <div
          style={{
            fontSize: 24,
            color: palette.mist,
            lineHeight: 1.35,
            fontWeight: 500,
          }}
        >
          {body}
        </div>
      </div>
    </div>
  );
};

const ArrowLine: React.FC<{
  from: { x: number; y: number };
  to: { x: number; y: number };
  color: string;
  delay?: number;
}> = ({ from, to, color, delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const progress = interpolate(
    frame,
    [fpsFromSeconds(delay, fps), fpsFromSeconds(delay + 0.8, fps)],
    [0, 1],
    {
      extrapolateLeft: "clamp",
      extrapolateRight: "clamp",
      easing: Easing.bezier(0.16, 1, 0.3, 1),
    },
  );
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const distance = Math.sqrt(dx * dx + dy * dy);
  const angle = (Math.atan2(dy, dx) * 180) / Math.PI;

  return (
    <div
      style={{
        position: "absolute",
        left: from.x,
        top: from.y,
        width: distance * progress,
        height: 4,
        borderRadius: 999,
        background: `linear-gradient(90deg, ${color}66, ${color})`,
        boxShadow: `0 0 22px ${color}55`,
        transformOrigin: "0 50%",
        transform: `rotate(${angle}deg)`,
      }}
    >
      <div
        style={{
          position: "absolute",
          right: -12,
          top: -6,
          width: 0,
          height: 0,
          borderTop: "8px solid transparent",
          borderBottom: "8px solid transparent",
          borderLeft: `14px solid ${color}`,
          opacity: progress > 0.9 ? 1 : 0,
        }}
      />
    </div>
  );
};

const BrowserWindow: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const reveal = entrance(frame, fps, 0.2);
  const cursorX = interpolate(frame, [0, fps * 3], [0, 260], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  return (
    <div
      style={{
        width: 760,
        height: 500,
        borderRadius: 34,
        padding: 22,
        border: `1px solid ${palette.line}`,
        background:
          "linear-gradient(180deg, rgba(15, 34, 52, 0.92), rgba(6, 17, 27, 0.9))",
        boxShadow: "0 26px 100px rgba(0,0,0,0.35)",
        transform: `translateY(${(1 - reveal) * 40}px) scale(${0.94 + reveal * 0.06})`,
        opacity: reveal,
        display: "flex",
        flexDirection: "column",
        gap: 18,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        {["#ff7b72", "#ffd36e", "#2ed3b7"].map((dot) => (
          <div
            key={dot}
            style={{
              width: 16,
              height: 16,
              borderRadius: "50%",
              background: dot,
            }}
          />
        ))}
        <div
          style={{
            marginLeft: 18,
            flex: 1,
            height: 42,
            borderRadius: 999,
            border: `1px solid ${palette.line}`,
            background: "rgba(255, 255, 255, 0.04)",
            display: "flex",
            alignItems: "center",
            paddingLeft: 24,
            fontSize: 24,
            color: palette.mist,
          }}
        >
          https://app.example.com/live-session
        </div>
      </div>
      <div
        style={{
          flex: 1,
          borderRadius: 26,
          border: `1px solid rgba(108, 231, 255, 0.14)`,
          background:
            "linear-gradient(160deg, rgba(13, 29, 44, 0.92), rgba(7, 17, 25, 0.9))",
          display: "flex",
          padding: 26,
          gap: 22,
          position: "relative",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            width: 160,
            borderRadius: 20,
            background: "rgba(255, 255, 255, 0.035)",
            display: "flex",
            flexDirection: "column",
            gap: 14,
            padding: 18,
          }}
        >
          {["Inbox", "Projects", "Reports", "Settings"].map((item, index) => (
            <div
              key={item}
              style={{
                height: 46,
                borderRadius: 14,
                background:
                  index === 1 ? "rgba(108, 231, 255, 0.16)" : "rgba(255,255,255,0.05)",
                display: "flex",
                alignItems: "center",
                paddingLeft: 18,
                color: palette.white,
                fontSize: 22,
                fontWeight: 600,
              }}
            >
              {item}
            </div>
          ))}
        </div>
        <div
          style={{
            flex: 1,
            display: "grid",
            gridTemplateColumns: "1.1fr 0.9fr",
            gap: 20,
          }}
        >
          <div
            style={{
              borderRadius: 22,
              background: "rgba(255, 255, 255, 0.04)",
              padding: 22,
              display: "flex",
              flexDirection: "column",
              gap: 16,
            }}
          >
            <div style={{ fontSize: 32, color: palette.white, fontWeight: 760 }}>
              Existing session. Real tabs. Real cookies.
            </div>
            <div style={{ fontSize: 23, color: palette.mist, lineHeight: 1.4 }}>
              No throwaway bot browser. RZN works inside the Chrome profile the user already
              depends on.
            </div>
            <div
              style={{
                marginTop: 12,
                padding: "18px 20px",
                borderRadius: 18,
                background: "rgba(46, 211, 183, 0.1)",
                border: "1px solid rgba(46, 211, 183, 0.22)",
                color: palette.teal,
                fontSize: 22,
                fontWeight: 720,
              }}
            >
              navigator.webdriver stays out of the picture.
            </div>
          </div>
          <div
            style={{
              borderRadius: 22,
              background: "rgba(255, 255, 255, 0.04)",
              padding: 22,
              position: "relative",
            }}
          >
            <div
              style={{
                fontSize: 26,
                color: palette.mist,
                marginBottom: 18,
                fontWeight: 700,
              }}
            >
              Trusted input only when it is actually needed
            </div>
            {Array.from({ length: 5 }).map((_, index) => (
              <div
                key={index}
                style={{
                  height: 42,
                  marginBottom: 12,
                  borderRadius: 14,
                  background: index === 1 ? "rgba(255, 211, 110, 0.16)" : "rgba(255,255,255,0.05)",
                }}
              />
            ))}
            <div
              style={{
                position: "absolute",
                left: 54 + cursorX,
                top: 188 + drift(frame, 12, 0.12),
                width: 34,
                height: 44,
                background: palette.white,
                clipPath: "polygon(0 0, 100% 58%, 62% 62%, 74% 100%, 48% 100%, 38% 68%, 0 68%)",
                boxShadow: "0 0 24px rgba(255,255,255,0.32)",
              }}
            />
          </div>
        </div>
      </div>
    </div>
  );
};

const IntroScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const shine = interpolate(frame, [0, fps * 4], [-540, 980], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const opacity = fadeWindow(frame, fps, 0, 4);

  return (
    <AbsoluteFill style={{ padding: 110, opacity }}>
      <div style={{ display: "flex", flex: 1, alignItems: "center", gap: 60 }}>
        <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 38 }}>
          <SectionHeading
            eyebrow="RZN Browser"
            title="Keep Chrome normal. Change the control path."
            subtitle="Stealth-first browser automation that runs in the real Chrome session you already use instead of booting a suspicious automation browser."
          />
          <div style={{ display: "flex", gap: 18 }}>
            <GlassCard
              title="Real session reuse"
              body="Tabs, cookies, login state, and extensions stay exactly where they already live."
              accent={palette.teal}
              width={350}
              height={250}
            />
            <GlassCard
              title="Deterministic or autonomous"
              body="Run a workflow step-by-step or let llm-auto plan against the same runtime."
              accent={palette.gold}
              width={350}
              height={250}
              style={{ marginTop: 44 }}
            />
          </div>
        </div>
        <div style={{ width: 760, position: "relative" }}>
          <BrowserWindow />
          <div
            style={{
              position: "absolute",
              inset: -30,
              background: "linear-gradient(112deg, transparent 0%, rgba(108, 231, 255, 0.2) 45%, transparent 70%)",
              transform: `translateX(${shine}px) rotate(8deg)`,
              opacity: 0.4,
              filter: "blur(8px)",
            }}
          />
        </div>
      </div>
    </AbsoluteFill>
  );
};

const RuntimeScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const opacity = fadeWindow(frame, fps, 0, 5);
  const flowReveal = entrance(frame, fps, 0.3);
  const cards = [
    { label: "Task", detail: "workflow or goal", x: 180, accent: palette.gold },
    { label: "RZN runtime", detail: "planner and step runner", x: 520, accent: palette.cyan },
    { label: "Native host", detail: "local bridge", x: 900, accent: palette.teal },
    { label: "Extension", detail: "Chrome MV3 control layer", x: 1260, accent: palette.coral },
    { label: "Live Chrome", detail: "existing tabs and profile", x: 1600, accent: palette.lime },
  ];

  return (
    <AbsoluteFill style={{ padding: 110, opacity }}>
      <SectionHeading
        eyebrow="How It Works"
        title="Everything important stays on the user’s machine."
        subtitle="The command flows through a local runtime, a native host, and the extension into the already-open Chrome session. No remote browser farm hiding in the middle."
      />
      <div style={{ flex: 1, position: "relative", marginTop: 70 }}>
        {cards.map((card, index) => {
          const y = 360 + (index % 2 === 0 ? 0 : 80);
          const reveal = entrance(frame, fps, 0.45 + index * 0.18);

          return (
            <div
              key={card.label}
              style={{
                position: "absolute",
                left: card.x,
                top: y,
                width: 250,
                height: 180,
                borderRadius: 26,
                padding: 24,
                display: "flex",
                flexDirection: "column",
                justifyContent: "space-between",
                background:
                  "linear-gradient(180deg, rgba(14, 32, 48, 0.92), rgba(7, 18, 28, 0.82))",
                border: `1px solid ${palette.line}`,
                transform: `translateY(${(1 - reveal) * 46}px) scale(${0.95 + reveal * 0.05})`,
                opacity: reveal,
                boxShadow: `0 18px 50px rgba(0,0,0,0.24), 0 0 0 1px ${card.accent}20 inset`,
              }}
            >
              <div
                style={{
                  width: 48,
                  height: 48,
                  borderRadius: 16,
                  background: `linear-gradient(135deg, ${card.accent}, rgba(255,255,255,0.12))`,
                }}
              />
              <div>
                <div
                  style={{
                    fontSize: 31,
                    color: palette.white,
                    fontWeight: 760,
                    lineHeight: 1.05,
                  }}
                >
                  {card.label}
                </div>
                <div
                  style={{
                    marginTop: 8,
                    fontSize: 22,
                    color: palette.mist,
                    lineHeight: 1.35,
                  }}
                >
                  {card.detail}
                </div>
              </div>
            </div>
          );
        })}

        <ArrowLine from={{ x: 430, y: 450 }} to={{ x: 520, y: 450 }} color={palette.gold} delay={0.85} />
        <ArrowLine from={{ x: 770, y: 530 }} to={{ x: 900, y: 530 }} color={palette.cyan} delay={1.05} />
        <ArrowLine from={{ x: 1150, y: 450 }} to={{ x: 1260, y: 450 }} color={palette.teal} delay={1.25} />
        <ArrowLine from={{ x: 1510, y: 530 }} to={{ x: 1600, y: 530 }} color={palette.coral} delay={1.45} />

        <div
          style={{
            position: "absolute",
            left: 590,
            top: 720,
            width: 760 * flowReveal,
            height: 120,
            borderRadius: 28,
            border: `1px solid ${palette.line}`,
            background: "rgba(8, 21, 33, 0.56)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            color: palette.white,
            fontSize: 36,
            fontWeight: 720,
            letterSpacing: -0.8,
            overflow: "hidden",
          }}
        >
          No extra browser window. No remote debugging port circus.
        </div>
      </div>
    </AbsoluteFill>
  );
};

const EscalationScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const opacity = fadeWindow(frame, fps, 0, 5);
  const highlight = interpolate(frame, [fps * 1.2, fps * 4], [0, 2.2], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const tracks = [
    {
      title: "DOM-first",
      body: "Same-origin pages get the light path: direct DOM and accessibility-aware targeting.",
      accent: palette.teal,
    },
    {
      title: "Scripted events",
      body: "If fidelity needs a bump, RZN escalates to richer in-page events without changing browsers.",
      accent: palette.gold,
    },
    {
      title: "Short CDP attach",
      body: "Only for hard cases: cross-origin iframes, trusted input, and stubborn UI that refuses to behave.",
      accent: palette.coral,
    },
  ];

  return (
    <AbsoluteFill style={{ padding: 110, opacity }}>
      <div style={{ display: "flex", gap: 70, flex: 1 }}>
        <div style={{ width: 720, display: "flex", flexDirection: "column", gap: 34 }}>
          <SectionHeading
            eyebrow="Escalation Ladder"
            title="RZN does not reach for CDP like a blunt instrument."
            subtitle="It starts with the lightest path that can possibly work, then escalates only when the page forces the issue."
          />
          <div
            style={{
              padding: "22px 26px",
              borderRadius: 22,
              background: "rgba(255,255,255,0.04)",
              border: `1px solid ${palette.line}`,
              color: palette.mist,
              fontSize: 27,
              lineHeight: 1.45,
            }}
          >
            That keeps common flows fast, keeps the browser looking normal, and avoids turning every
            click into a full-control-plane drama.
          </div>
        </div>

        <div style={{ flex: 1, position: "relative", display: "flex", alignItems: "center" }}>
          <div
            style={{
              position: "absolute",
              left: 118,
              top: 160,
              bottom: 160,
              width: 6,
              borderRadius: 999,
              background: "rgba(255,255,255,0.08)",
            }}
          />
          {tracks.map((track, index) => {
            const reveal = entrance(frame, fps, 0.35 + index * 0.25);
            const top = 150 + index * 250;
            const active = highlight >= index && highlight < index + 1;

            return (
              <div
                key={track.title}
                style={{
                  position: "absolute",
                  left: 0,
                  top,
                  display: "flex",
                  alignItems: "center",
                  gap: 30,
                  transform: `translateY(${(1 - reveal) * 32}px)`,
                  opacity: reveal,
                }}
              >
                <div
                  style={{
                    width: 240,
                    display: "flex",
                    justifyContent: "center",
                  }}
                >
                  <div
                    style={{
                      width: 92,
                      height: 92,
                      borderRadius: 28,
                      background: `linear-gradient(135deg, ${track.accent}, rgba(255,255,255,0.12))`,
                      boxShadow: `0 0 46px ${track.accent}${active ? "66" : "24"}`,
                      opacity: active ? 1 : 0.36,
                    }}
                  />
                </div>
                <div
                  style={{
                    width: 760,
                    padding: 30,
                    borderRadius: 28,
                    background:
                      active
                        ? "linear-gradient(180deg, rgba(21, 44, 62, 0.95), rgba(10, 20, 32, 0.9))"
                        : "linear-gradient(180deg, rgba(12, 28, 42, 0.86), rgba(7, 16, 25, 0.82))",
                    border: `1px solid ${track.accent}${active ? "88" : "26"}`,
                    boxShadow: active ? `0 0 80px ${track.accent}22` : "none",
                  }}
                >
                  <div style={{ color: palette.white, fontSize: 38, fontWeight: 780 }}>
                    {track.title}
                  </div>
                  <div
                    style={{
                      marginTop: 12,
                      color: palette.mist,
                      fontSize: 25,
                      lineHeight: 1.42,
                    }}
                  >
                    {track.body}
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </AbsoluteFill>
  );
};

const ModesScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const opacity = fadeWindow(frame, fps, 0, 5);
  const merge = interpolate(frame, [fps * 1.6, fps * 4.2], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  return (
    <AbsoluteFill style={{ padding: 110, opacity }}>
      <SectionHeading
        eyebrow="Two Entry Points"
        title="Workflows and agent mode use the same execution stack."
        subtitle="That matters because you can start deterministic, keep tight control, and graduate to goal-driven runs without swapping tools or changing browsers."
        align="center"
      />
      <div style={{ flex: 1, position: "relative", marginTop: 60 }}>
        <GlassCard
          title="Workflow mode"
          body="JSON steps for explicit, repeatable automation. Great when the run should behave the same way every time."
          accent={palette.teal}
          width={420}
          height={260}
          style={{
            position: "absolute",
            left: 210 - merge * 90,
            top: 250,
          }}
        />
        <GlassCard
          title="Agent mode"
          body="Goal string plus planner loop. The model reasons about the next step, then sends it through the same browser bridge."
          accent={palette.gold}
          width={420}
          height={260}
          style={{
            position: "absolute",
            right: 210 - merge * 90,
            top: 250,
          }}
        />
        <ArrowLine from={{ x: 630, y: 380 }} to={{ x: 860, y: 480 }} color={palette.teal} delay={0.8} />
        <ArrowLine from={{ x: 1290, y: 380 }} to={{ x: 1060, y: 480 }} color={palette.gold} delay={1} />
        <div
          style={{
            position: "absolute",
            left: 720,
            top: 420,
            width: 480,
            height: 250,
            borderRadius: 30,
            padding: 32,
            background:
              "linear-gradient(180deg, rgba(17, 41, 60, 0.96), rgba(7, 17, 27, 0.92))",
            border: `1px solid ${palette.line}`,
            boxShadow: "0 26px 80px rgba(0,0,0,0.26)",
            transform: `scale(${0.9 + merge * 0.1})`,
            opacity: Math.max(merge, 0.12),
          }}
        >
          <div style={{ color: palette.white, fontSize: 42, fontWeight: 800 }}>
            Shared browser bridge
          </div>
          <div
            style={{
              marginTop: 14,
              color: palette.mist,
              fontSize: 26,
              lineHeight: 1.42,
            }}
          >
            Same step runner. Same extension. Same session reuse. Different input surface.
          </div>
          <div style={{ display: "flex", gap: 14, marginTop: 28 }}>
            {["navigate", "click", "fill", "extract"].map((step) => (
              <div
                key={step}
                style={{
                  padding: "12px 16px",
                  borderRadius: 16,
                  background: "rgba(255,255,255,0.06)",
                  color: palette.white,
                  fontSize: 20,
                  fontWeight: 700,
                }}
              >
                {step}
              </div>
            ))}
          </div>
        </div>
      </div>
    </AbsoluteFill>
  );
};

const ClosingScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const opacity = fadeWindow(frame, fps, 0, 6);
  const pulse = 0.96 + Math.sin(frame * 0.08) * 0.015;
  const tags = [
    "Google and workflow catalog runs",
    "Logged-in SaaS operations",
    "Cross-origin iframe handling",
    "Profile-safe local agent tasks",
  ];

  return (
    <AbsoluteFill style={{ padding: 110, opacity, justifyContent: "center" }}>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          textAlign: "center",
        }}
      >
        <div
          style={{
            width: 200,
            height: 200,
            borderRadius: 48,
            background: `linear-gradient(135deg, ${palette.cyan}, ${palette.teal})`,
            boxShadow: "0 0 120px rgba(108, 231, 255, 0.28)",
            transform: `scale(${pulse})`,
            marginBottom: 40,
          }}
        />
        <SectionHeading
          eyebrow="Bottom Line"
          title="RZN is for browser automation that has to survive contact with reality."
          subtitle="Use the real Chrome session, keep the control path light, and escalate only when the page earns it."
          align="center"
        />
        <div
          style={{
            display: "flex",
            flexWrap: "wrap",
            justifyContent: "center",
            gap: 16,
            marginTop: 44,
            maxWidth: 1220,
          }}
        >
          {tags.map((tag, index) => {
            const reveal = entrance(frame, fps, 0.35 + index * 0.12);

            return (
              <div
                key={tag}
                style={{
                  padding: "18px 24px",
                  borderRadius: 999,
                  background: "rgba(255,255,255,0.05)",
                  border: `1px solid ${palette.line}`,
                  color: palette.white,
                  fontSize: 26,
                  fontWeight: 640,
                  transform: `translateY(${(1 - reveal) * 18}px)`,
                  opacity: reveal,
                }}
              >
                {tag}
              </div>
            );
          })}
        </div>
      </div>
    </AbsoluteFill>
  );
};

export const RznBrowserExplainer: React.FC = () => {
  return (
    <AbsoluteFill>
      <Background />
      <Sequence durationInFrames={120}>
        <IntroScene />
      </Sequence>
      <Sequence from={120} durationInFrames={150}>
        <RuntimeScene />
      </Sequence>
      <Sequence from={270} durationInFrames={150}>
        <EscalationScene />
      </Sequence>
      <Sequence from={420} durationInFrames={150}>
        <ModesScene />
      </Sequence>
      <Sequence from={570} durationInFrames={150}>
        <ClosingScene />
      </Sequence>
    </AbsoluteFill>
  );
};
