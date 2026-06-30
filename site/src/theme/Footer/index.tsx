import React from 'react';
import styles from './footer.module.css';

const COLS = [
  {h: 'Product', links: [['Install', '/docs/getting-started/install'], ['Quickstart', '/docs/getting-started/quickstart'], ['Authoring', '/docs/authoring/overview'], ['Changelog', 'https://github.com/devrelaicom/tome/blob/main/CHANGELOG.md']]},
  {h: 'Project', links: [['GitHub', 'https://github.com/devrelaicom/tome'], ['Security', '/docs/reference/security-model'], ['Exit codes', '/docs/reference/exit-codes']]},
  {h: 'Midnight', links: [['Midnight Network', 'https://midnight.network'], ['Midnight Expert', 'https://midnightntwrk.expert'], ['Compact', 'https://docs.midnight.network']]},
];

export default function Footer(): React.JSX.Element {
  return (
    <footer className={styles.footer}>
      <div className={styles.wrap}>
        <div className={styles.grid}>
          <div>
            <div className={styles.wm}>TOME</div>
            <div className={styles.made}>A Midnight Foundation tool.<br/>tome-mcp.com<br/>MIT OR Apache-2.0</div>
          </div>
          {COLS.map((col) => (
            <div key={col.h}>
              <h4>{col.h}</h4>
              {col.links.map(([label, href]) => <a key={label} href={href}>{label}</a>)}
            </div>
          ))}
        </div>
        <div className={styles.bar}>
          <span>© 2026 Midnight Foundation</span>
          <span>EX LIBRIS — kept on your machine</span>
        </div>
      </div>
    </footer>
  );
}
