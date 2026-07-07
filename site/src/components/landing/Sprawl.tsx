import React, {useEffect, useLayoutEffect, useRef, useState} from 'react';
import styles from './landing.module.css';

type Row = {harness: string; path: string; version: number; badge: string};

// The five copies the section opens on — the same drift the screenshot shows.
const INITIAL_ROWS: Row[] = [
  {harness: 'Claude Code', path: '.claude/skills/', version: 1, badge: 'v1'},
  {harness: 'Cursor', path: '.cursor/rules/', version: 3, badge: 'v3'},
  {harness: 'Codex', path: 'AGENTS.md', version: 2, badge: 'v2 (edited)'},
  {harness: 'Gemini CLI', path: 'GEMINI.md', version: 2, badge: 'v2 !!STALE!!'},
  {harness: 'OpenCode', path: 'AGENTS.md', version: 1, badge: 'v1 (conflicted copy)'},
];

// The harnesses a churned row can be replaced with, each with its rules sink.
const POOL: {harness: string; path: string}[] = [
  {harness: 'Claude Code', path: '.claude/skills/'},
  {harness: 'Cursor', path: '.cursor/rules/'},
  {harness: 'Codex', path: 'AGENTS.md'},
  {harness: 'Gemini CLI', path: 'GEMINI.md'},
  {harness: 'OpenCode', path: 'AGENTS.md'},
  {harness: 'GitHub Copilot', path: '.github/copilot-instructions.md'},
  {harness: 'Copilot CLI', path: '.github/copilot-instructions.md'},
  {harness: 'Zed', path: '.rules'},
  {harness: 'Devin', path: 'AGENTS.md'},
  {harness: 'Cline', path: '.clinerules/'},
  {harness: 'JetBrains AI', path: '.aiassistant/rules/'},
  {harness: 'Junie', path: '.junie/AGENTS.md'},
  {harness: 'Kiro', path: '.kiro/steering/'},
  {harness: 'Crush', path: 'CRUSH.md'},
  {harness: 'Antigravity', path: '.agent/rules/'},
  {harness: 'Pi', path: 'AGENTS.md'},
  {harness: 'Goose', path: 'AGENTS.md'},
];

// Ten-odd ways a hand-edited copy drifts from a clean version tag. Each takes
// the current version string ("v2") and mangles it.
const MARKERS: ((v: string) => string)[] = [
  (v) => `${v} (edited)`,
  (v) => `${v}.bak`,
  (v) => `${v} !!STALE!!`,
  (v) => `${v}-final`,
  (v) => `${v}-final-final`,
  (v) => `${v} (Aaron's edit)`,
  (v) => `${v} (do not use)`,
  (v) => `${v}.orig`,
  (v) => `${v} (WIP)`,
  (v) => `${v} (conflicted copy)`,
  (v) => `${v}~`,
];

const CARD_FADE = 450;
const CASCADE_STEP = 140;
const DRIFT_MIN = 2000;
const DRIFT_MAX = 5000;
const CHURN_MIN = 5000;
const CHURN_MAX = 10000;

const randInt = (lo: number, hi: number) => lo + Math.floor(Math.random() * (hi - lo + 1));
const fmtV = (n: number) => (Number.isInteger(n) ? String(n) : n.toFixed(1));
const isMarker = (r: Row) => r.badge !== `v${fmtV(r.version)}`;

// useLayoutEffect on the client (to hide cards before first paint), useEffect on
// the server (React warns about useLayoutEffect during SSR).
const useIsoLayoutEffect = typeof window !== 'undefined' ? useLayoutEffect : useEffect;

// Scroll fade-in for the prose block; no-op under prefers-reduced-motion.
function useFadeIn(): React.RefObject<HTMLDivElement | null> {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    el.classList.add(styles.fadeIn);
    const io = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) {
        el.classList.add(styles.fadeInShown);
        io.disconnect();
      }
    }, {threshold: 0.2});
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return ref;
}

export default function Sprawl(): React.JSX.Element {
  const textRef = useFadeIn();
  const colRef = useRef<HTMLDivElement>(null);
  const [rows, setRows] = useState<Row[]>(INITIAL_ROWS);
  const [shown, setShown] = useState<boolean[]>(() => INITIAL_ROWS.map(() => false));
  const [animate, setAnimate] = useState(false);

  // Hide the cards before the first paint (only when motion is allowed), so the
  // cascade starts from empty rather than flashing the whole set first.
  useIsoLayoutEffect(() => {
    if (typeof window === 'undefined') return;
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    setAnimate(true);
  }, []);

  useEffect(() => {
    if (!animate) return;
    const el = colRef.current;
    if (!el) return;

    let cancelled = false;
    let started = false;
    const timers: number[] = [];
    const push = (fn: () => void, ms: number) => {
      const t = window.setTimeout(fn, ms);
      timers.push(t);
      return t;
    };

    // One row drifts: bump the version (whole or minor), then 1-in-5 chance it
    // also picks up a messy hand-edit marker. Reschedules itself 2-5s later.
    const scheduleDrift = (slot: number) =>
      push(() => {
        if (cancelled) return;
        setRows((prev) => {
          const r = prev[slot];
          if (!r) return prev;
          const version =
            Math.random() < 0.5
              ? Math.floor(r.version) + 1
              : Math.round((r.version + 0.1) * 10) / 10;
          const vstr = `v${fmtV(version)}`;
          const badge = Math.random() < 0.2 ? MARKERS[randInt(0, MARKERS.length - 1)](vstr) : vstr;
          const next = [...prev];
          next[slot] = {...r, version, badge};
          return next;
        });
        scheduleDrift(slot);
      }, randInt(DRIFT_MIN, DRIFT_MAX));

    // Every 5-10s, one row fades out and returns as a different harness — one
    // not currently on screen — starting fresh at v1.
    const scheduleChurn = () =>
      push(() => {
        if (cancelled) return;
        const slot = randInt(0, INITIAL_ROWS.length - 1);
        setShown((s) => s.map((v, i) => (i === slot ? false : v)));
        push(() => {
          if (cancelled) return;
          setRows((prev) => {
            const onScreen = new Set(prev.map((r) => r.harness));
            const candidates = POOL.filter((p) => !onScreen.has(p.harness));
            if (candidates.length === 0) return prev;
            const pick = candidates[randInt(0, candidates.length - 1)];
            const next = [...prev];
            next[slot] = {harness: pick.harness, path: pick.path, version: 1, badge: 'v1'};
            return next;
          });
          setShown((s) => s.map((v, i) => (i === slot ? true : v)));
          scheduleChurn();
        }, CARD_FADE);
      }, randInt(CHURN_MIN, CHURN_MAX));

    const startMotion = () => {
      if (started) return;
      started = true;
      INITIAL_ROWS.forEach((_, i) => push(() => setShown((s) => s.map((v, idx) => (idx === i ? true : v))), CASCADE_STEP * i));
      INITIAL_ROWS.forEach((_, i) => scheduleDrift(i));
      scheduleChurn();
    };

    const io = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) {
        startMotion();
        io.disconnect();
      }
    }, {threshold: 0.3});
    io.observe(el);

    return () => {
      cancelled = true;
      io.disconnect();
      timers.forEach((t) => clearTimeout(t));
    };
  }, [animate]);

  return (
    <section className={`${styles.section} ${styles.sectionDeep}`}>
      <div className={styles.wrap}>
        <div className={styles.kicker}>THE SPRAWL</div>
        <div className={styles.split}>
          <div ref={textRef}>
            <h2 className={styles.h2}>Five agents. Five dialects. One you.</h2>
            <p className={styles.lede}>
              Your prompts, rules, and skills are pasted into <code>.claude/</code>,{' '}
              <code>.cursor/rules</code>, <code>AGENTS.md</code>, <code>GEMINI.md</code> — each
              copy drifting from the last. And the ones you do load sit in the context window all
              day, billing you for knowledge the agent isn&rsquo;t using.
            </p>
          </div>
          <div ref={colRef} className={styles.sprawlCol}>
            {rows.map((r, i) => (
              <div
                key={i}
                className={styles.sprawlCard}
                style={
                  animate
                    ? {
                        opacity: shown[i] ? 1 : 0,
                        transform: shown[i] ? 'none' : 'translateY(10px)',
                        transition: `opacity ${CARD_FADE}ms ease, transform ${CARD_FADE}ms ease`,
                      }
                    : undefined
                }>
                <div>
                  <div className={styles.sprawlPath}>{r.harness} · {r.path}</div>
                  <div className={styles.sprawlFile}>└─ review-checklist/SKILL.md</div>
                </div>
                <span
                  className={`${styles.sprawlBadge}${isMarker(r) ? ` ${styles.sprawlBadgeDrift}` : ''}`}>
                  {r.badge}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
