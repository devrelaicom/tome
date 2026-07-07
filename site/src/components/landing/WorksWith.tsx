import React, {useEffect, useState} from 'react';
import styles from './landing.module.css';

// The full set of configured harnesses, shown five at a time. The first group
// is the familiar five (so the strip reads the same at a glance as it always
// has), then the breadth reveals itself. Kept in sync with the harness registry
// (docs/using-tome/harnesses.md).
const GROUPS: string[][] = [
  ['Claude Code', 'Cursor', 'Codex', 'Gemini CLI', 'OpenCode'],
  ['GitHub Copilot', 'Copilot CLI', 'Zed', 'Devin', 'Cline'],
  ['JetBrains AI', 'Junie', 'Kiro', 'Crush', 'Antigravity'],
  ['Pi', 'Goose'],
];
const MCP_LINE = 'and any harness that supports MCP';

// Timings (ms). All motion is skipped under prefers-reduced-motion.
const PAINT = 60; // let a hidden group render before fading it in
const STAGGER_IN = 300; // gap between each item fading in, left to right
const HOLD = 2000; // pause once the whole group is shown
const STAGGER_OUT = 200; // gap between each item fading out, left to right
const GROUP_GAP = 320; // beat between groups
const FADE = 650; // opacity transition duration
const FINALE_HOLD = 3000; // pause on "…any harness that supports MCP"

const nbsp = (s: string) => s.replace(/ /g, ' ');

export default function WorksWith(): React.JSX.Element {
  const [reduced, setReduced] = useState(false);
  const [phase, setPhase] = useState<'group' | 'finale'>('group');
  const [items, setItems] = useState<string[]>(GROUPS[0]);
  const [visible, setVisible] = useState<boolean[]>(() => GROUPS[0].map(() => false));
  const [finaleShown, setFinaleShown] = useState(false);

  useEffect(() => {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      setReduced(true);
      return;
    }

    let cancelled = false;
    const timers: number[] = [];
    const sleep = (ms: number) =>
      new Promise<void>((res) => timers.push(window.setTimeout(res, ms)));

    const run = async () => {
      while (!cancelled) {
        for (let g = 0; g < GROUPS.length; g++) {
          const group = GROUPS[g];
          const isLast = g === GROUPS.length - 1;

          setPhase('group');
          setItems(group);
          setVisible(group.map(() => false));
          await sleep(PAINT);
          if (cancelled) return;

          for (let i = 0; i < group.length; i++) {
            setVisible((v) => v.map((on, idx) => (idx === i ? true : on)));
            await sleep(STAGGER_IN);
            if (cancelled) return;
          }

          await sleep(HOLD);
          if (cancelled) return;

          if (isLast) {
            // The final group leaves all at once, then hands off to the finale.
            setVisible(group.map(() => false));
            await sleep(FADE);
          } else {
            for (let i = 0; i < group.length; i++) {
              setVisible((v) => v.map((on, idx) => (idx === i ? false : on)));
              await sleep(STAGGER_OUT);
              if (cancelled) return;
            }
            await sleep(GROUP_GAP);
          }
          if (cancelled) return;
        }

        setPhase('finale');
        setFinaleShown(false);
        await sleep(PAINT);
        if (cancelled) return;
        setFinaleShown(true);
        await sleep(FADE + FINALE_HOLD);
        if (cancelled) return;
        setFinaleShown(false);
        await sleep(FADE);
      }
    };

    void run();
    return () => {
      cancelled = true;
      timers.forEach((t) => clearTimeout(t));
    };
  }, []);

  return (
    <div className={styles.works} role="img" aria-label="Tome works with Claude Code, Cursor, Codex, Gemini CLI, OpenCode, and any harness that supports MCP">
      <span className={styles.worksLbl}>works with</span>
      {reduced ? (
        <>
          {GROUPS[0].map((name, i) => (
            <React.Fragment key={name}>
              {i > 0 && <span className={styles.worksSep}>/</span>}
              <span className={styles.worksItem}>{nbsp(name)}</span>
            </React.Fragment>
          ))}
          <span className={styles.worksSep}>·</span>
          <span className={styles.worksMcp}>{MCP_LINE}</span>
        </>
      ) : phase === 'finale' ? (
        <span
          className={styles.worksMcp}
          style={{opacity: finaleShown ? 1 : 0, transition: `opacity ${FADE}ms ease`}}>
          {MCP_LINE}
        </span>
      ) : (
        items.map((name, i) => (
          <span
            key={i}
            className={styles.worksUnit}
            style={{
              opacity: visible[i] ? 1 : 0,
              transform: visible[i] ? 'none' : 'translateY(3px)',
              transition: `opacity ${FADE}ms ease, transform ${FADE}ms ease`,
            }}>
            {i > 0 && <span className={styles.worksSep}>/</span>}
            <span className={styles.worksItem}>{nbsp(name)}</span>
          </span>
        ))
      )}
    </div>
  );
}
