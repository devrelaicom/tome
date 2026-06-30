import React, {useEffect, useRef} from 'react';
import styles from './landing.module.css';

// Excerpted from specs/reference/captures/convert.txt and lint.txt (tome 0.6.0
// against the midnight-expert marketplace). `⋮` marks elided lines and the
// Done line's local output path is elided per the NUMBERS.md slug rule; the
// counts (616 files / 128 warnings / 100 infos; 0 errors / 18 warnings) are
// the captured figures.
const SESSION: {kind: 'cmd' | 'out' | 'ok'; text: string}[] = [
  {kind: 'cmd', text: 'tome catalog convert ~/.claude/plugins/marketplaces/midnight-expert --output midnight-expert-tome'},
  {kind: 'out', text: 'Converted claude-code catalog `midnight-expert` → `midnight-expert-tome`'},
  {kind: 'out', text: '  [warning] harness-ism/claude-skill-dir: rewrote 6 occurrence(s) of `${CLAUDE_SKILL_DIR}` → `${TOME_SKILL_DIR}`'},
  {kind: 'out', text: '  [info] convert/dropped-manifest-field: plugin.json `homepage` is not modelled by Tome; dropping it'},
  {kind: 'out', text: '  ⋮'},
  {kind: 'ok', text: 'Done: 616 file(s) … (128 warning(s), 100 info(s))'},
  {kind: 'cmd', text: 'tome catalog lint ./midnight-expert-tome'},
  {kind: 'out', text: '[warning] lint/description-too-long: entry `compact-dev` description is 2557 characters (max 1024)'},
  {kind: 'out', text: '  ⋮'},
  {kind: 'ok', text: 'Summary: 0 error(s), 18 warning(s), 0 info(s)'},
];

// Scroll fade-in for this section; no-op under prefers-reduced-motion.
function useFadeIn(): React.RefObject<HTMLElement | null> {
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    el.classList.add(styles.fadeIn);
    const io = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) {
        el.classList.add(styles.fadeInShown);
        io.disconnect();
      }
    }, {threshold: 0.15});
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return ref;
}

export default function AuthoringOnramp(): React.JSX.Element {
  const ref = useFadeIn();
  return (
    <section ref={ref} className={styles.section}>
      <div className={styles.wrap}>
        <div className={styles.kicker}>CHAPTER III — WRITE YOUR OWN</div>
        <div className={styles.split}>
          <div>
            <h2 className={styles.h2}>Already have plugins? Convert them.</h2>
            <p className={styles.lede}>
              Bring a Claude Code marketplace, a Cursor rules folder, or a loose pile of{' '}
              <code>SKILL.md</code> files — <code>tome convert</code> turns them into a catalog.{' '}
              <code>create</code> scaffolds new ones; <code>lint</code> keeps you publishable.
            </p>
            <p>
              <a className={styles.btn} href="/docs/authoring/convert">Read the conversion guide →</a>
            </p>
          </div>
          <div className={`${styles.terminal} ${styles.terminalStatic}`} role="img" aria-label="fig. 2 — convert, then lint">
            <div className={styles.termTitle}>fig. 2 — convert, then lint</div>
            {SESSION.map((l, i) => (
              <div key={i} className={styles[`term_${l.kind}`]}>
                {l.kind === 'cmd' ? <><span className={styles.termPrompt}>$ </span>{l.text}</> : l.text}
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
