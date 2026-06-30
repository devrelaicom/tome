import React, {useEffect, useRef, useState} from 'react';
import styles from './landing.module.css';

export type TermLine = {kind: 'cmd' | 'out' | 'ok'; text: string};

// Auto-typing looping terminal. Honors prefers-reduced-motion (renders the
// full transcript statically) and only starts when scrolled into view.
export default function TypedTerminal({lines, label}: {lines: TermLine[]; label: string}): React.JSX.Element {
  const [shown, setShown] = useState<TermLine[]>([]);
  const [partial, setPartial] = useState('');
  const [reduced, setReduced] = useState(false);
  const [started, setStarted] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    setReduced(mq.matches);
    const onChange = (e: MediaQueryListEvent) => setReduced(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const io = new IntersectionObserver(([entry]) => entry.isIntersecting && setStarted(true), {threshold: 0.4});
    io.observe(el);
    return () => io.disconnect();
  }, []);

  useEffect(() => {
    if (reduced || !started) return;
    let line = 0;
    let char = 0;
    let cancelled = false;
    const timers: number[] = [];
    const later = (fn: () => void, ms: number) => timers.push(window.setTimeout(fn, ms));
    const step = () => {
      if (cancelled) return;
      if (line >= lines.length) {
        later(() => { setShown([]); setPartial(''); line = 0; char = 0; step(); }, 3500);
        return;
      }
      const cur = lines[line];
      if (cur.kind === 'cmd' && char < cur.text.length) {
        char += 1;
        setPartial(cur.text.slice(0, char));
        later(step, 28);
      } else {
        setShown((prev) => [...prev, cur]);
        setPartial('');
        line += 1;
        char = 0;
        later(step, cur.kind === 'cmd' ? 350 : 650);
      }
    };
    step();
    return () => { cancelled = true; timers.forEach(clearTimeout); };
  }, [reduced, started, lines]);

  const render = (l: TermLine, i: number) => (
    <div key={i} className={styles[`term_${l.kind}`]}>{l.kind === 'cmd' ? <><span className={styles.termPrompt}>$ </span>{l.text}</> : l.text}</div>
  );

  return (
    <div ref={rootRef} className={styles.terminal} role="img" aria-label={label}>
      <div className={styles.termTitle}>{label}</div>
      {(reduced ? lines : shown).map(render)}
      {!reduced && partial && <div className={styles.term_cmd}><span className={styles.termPrompt}>$ </span>{partial}<span className={styles.termCursor} /></div>}
    </div>
  );
}
