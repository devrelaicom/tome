import React from 'react';
import styles from './landing.module.css';
import TypedTerminal, {type TermLine} from './TypedTerminal';

// Every number and score below is read from specs/reference/captures/
// (catalog-add.txt, query.txt, status.txt, NUMBERS.md) — captured from a real
// tome 0.6.0 run against the midnight-expert catalog, never invented.
// NUMBERS.md records no token measurement for the top hit, so the closing
// line quotes the captured word count instead.
const DEMO: TermLine[] = [
  {kind: 'cmd', text: 'tome catalog add devrelaicom/midnight-expert-tome'},
  {kind: 'ok', text: '✓ cloned · 13 plugins · 28 entries indexed'},
  {kind: 'cmd', text: 'tome query "verify a Compact contract"'},
  {kind: 'ok', text: '→ 4.7874  midnight-verify:verify-by-execution'},
  {kind: 'ok', text: '→ 3.4658  midnight-verify:verify-by-zkir-checker'},
  {kind: 'ok', text: '→ 3.1529  midnight-verify:verify-compact'},
  {kind: 'out', text: 'loaded 1 skill · 1,539 words — the other 27 stayed on the shelf'},
];

export default function Hero(): React.JSX.Element {
  return (
    <section className={styles.section}>
      <div className={`${styles.wrap} ${styles.heroInner}`}>
        <div className={styles.kicker}>CHAPTER I — IN WHICH YOUR AGENTS LEARN TO READ</div>
        <h1 className={styles.h1}>Tome<span className={styles.stop}>.</span></h1>
        <p className={styles.lede}>
          One bookshelf for everything your coding agents know — read everywhere, loaded one
          skill at a time.
        </p>
        <div className={styles.ctas}>
          <a className={`${styles.btn} ${styles.btnSolid}`} href="/docs/getting-started/install">▸ Install Tome</a>
          <a className={styles.btn} href="/docs/getting-started/quickstart">Read the docs</a>
        </div>
        <TypedTerminal label="fig. 1 — the bookshelf at work" lines={DEMO} />
        <div className={styles.works}>
          <span className={styles.worksLbl}>works with</span>
          <span className={styles.worksItem}>Claude&nbsp;Code</span><span className={styles.worksSep}>/</span>
          <span className={styles.worksItem}>Cursor</span><span className={styles.worksSep}>/</span>
          <span className={styles.worksItem}>Codex</span><span className={styles.worksSep}>/</span>
          <span className={styles.worksItem}>Gemini&nbsp;CLI</span><span className={styles.worksSep}>/</span>
          <span className={styles.worksItem}>OpenCode</span>
        </div>
      </div>
    </section>
  );
}
