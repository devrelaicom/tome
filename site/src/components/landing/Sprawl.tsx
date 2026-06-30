import React, {useEffect, useRef} from 'react';
import styles from './landing.module.css';

// One copy of the same skill per harness, each drifting from the last.
const COPIES: {harness: string; path: string; badge: string}[] = [
  {harness: 'Claude Code', path: '.claude/skills/', badge: 'v1'},
  {harness: 'Cursor', path: '.cursor/rules/', badge: 'v3'},
  {harness: 'Codex', path: 'AGENTS.md', badge: 'v2 (edited)'},
  {harness: 'Gemini CLI', path: 'GEMINI.md', badge: 'stale'},
  {harness: 'OpenCode', path: 'AGENTS.md', badge: '?'},
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

export default function Sprawl(): React.JSX.Element {
  const ref = useFadeIn();
  return (
    <section ref={ref} className={`${styles.section} ${styles.sectionDeep}`}>
      <div className={styles.wrap}>
        <div className={styles.kicker}>THE SPRAWL</div>
        <div className={styles.split}>
          <div>
            <h2 className={styles.h2}>Five agents. Five dialects. One you.</h2>
            <p className={styles.lede}>
              Your prompts, rules, and skills are pasted into <code>.claude/</code>,{' '}
              <code>.cursor/rules</code>, <code>AGENTS.md</code>, <code>GEMINI.md</code> — each
              copy drifting from the last. And the ones you do load sit in the context window all
              day, billing you for knowledge the agent isn&rsquo;t using.
            </p>
          </div>
          <div className={styles.sprawlCol}>
            {COPIES.map((c) => (
              <div key={c.harness} className={styles.sprawlCard}>
                <div>
                  <div className={styles.sprawlPath}>{c.harness} · {c.path}</div>
                  <div className={styles.sprawlFile}>└─ review-checklist/SKILL.md</div>
                </div>
                <span className={styles.sprawlBadge}>{c.badge}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
