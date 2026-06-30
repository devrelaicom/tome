import React, {useEffect, useRef} from 'react';
import styles from './landing.module.css';

const PLATES: {term: string; desc: string}[] = [
  {term: 'Local, entirely', desc: 'index, embeddings & reranker on your disk — no telemetry, no cloud'},
  {term: 'One binary, in Rust', desc: 'a single static binary; semantic search built in'},
  {term: 'Honest exits', desc: 'every failure has its own exit code — scriptable, CI-friendly'},
  {term: 'Five harnesses', desc: 'native config for each — rules, MCP, agents, hooks'},
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

export default function Colophon(): React.JSX.Element {
  const ref = useFadeIn();
  return (
    <section ref={ref} className={`${styles.section} ${styles.sectionDeep}`}>
      <div className={styles.wrap}>
        <div className={styles.kicker}>COLOPHON — SPECIFICATIONS OF THIS VOLUME</div>
        <div className={`${styles.plates} ${styles.platesFour}`}>
          {PLATES.map((p) => (
            <div key={p.term} className={styles.plate}>
              <div className={styles.plateTerm}>{p.term}</div>
              <p className={styles.plateDesc}>{p.desc}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
