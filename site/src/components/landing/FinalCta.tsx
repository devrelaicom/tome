import React, {useEffect, useRef} from 'react';
import styles from './landing.module.css';

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

export default function FinalCta(): React.JSX.Element {
  const ref = useFadeIn();
  return (
    <section ref={ref} className={styles.cta}>
      <div className={styles.wrap}>
        <div className={styles.ctaKicker}>BEGIN</div>
        <h2 className={styles.ctaTitle}>Add it to your bookshelf.</h2>
        <code className={styles.install}>brew install aaronbassett/homebrew-tap/tome</code>
        <div className={styles.installSub}>or cargo install tome-mcp</div>
        <a className={styles.btnGhost} href="/docs/getting-started/quickstart">Quickstart → tome init</a>
      </div>
    </section>
  );
}
