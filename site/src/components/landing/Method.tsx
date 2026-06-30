import React, {useEffect, useRef} from 'react';
import styles from './landing.module.css';

const PLATES: {num: string; label: string; cmd: string}[] = [
  {num: '①', label: 'ACQUIRE', cmd: 'tome catalog add <repo>'},
  {num: '②', label: 'BIND', cmd: 'tome harness use cursor'},
  {num: '③', label: 'CONSULT', cmd: 'tome query "…"'},
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

export default function Method(): React.JSX.Element {
  const ref = useFadeIn();
  return (
    <section ref={ref} className={styles.section}>
      <div className={styles.wrap}>
        <div className={styles.kicker}>CHAPTER II — THE METHOD</div>
        <h2 className={styles.h2}>Shelve it once. Read it everywhere.</h2>
        <div className={styles.plates}>
          {PLATES.map((p) => (
            <div key={p.label} className={styles.plate}>
              <div className={styles.plateNum}>{p.num} {p.label}</div>
              <code className={styles.plateCmd}>{p.cmd}</code>
            </div>
          ))}
        </div>
        <p className={styles.closing}>
          Inside a harness the same search runs over MCP — the agent pulls exactly the skill it
          needs, mid-task, by itself.
        </p>
      </div>
    </section>
  );
}
