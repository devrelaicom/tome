import React from 'react';
import Layout from '@theme/Layout';
import Hero from '@site/src/components/landing/Hero';
import Sprawl from '@site/src/components/landing/Sprawl';
import Method from '@site/src/components/landing/Method';
import Colophon from '@site/src/components/landing/Colophon';
import AuthoringOnramp from '@site/src/components/landing/AuthoringOnramp';
import FinalCta from '@site/src/components/landing/FinalCta';

export default function Home(): React.JSX.Element {
  return (
    <Layout
      title="Tome — One bookshelf, every coding agent"
      description="One bookshelf for everything your coding agents know — read everywhere, loaded one skill at a time. Tome manages skills, commands, agents & hooks across Claude Code, Cursor, Codex, Gemini & OpenCode.">
      <main>
        <Hero />
        <Sprawl />
        <Method />
        <Colophon />
        <AuthoringOnramp />
        <FinalCta />
      </main>
    </Layout>
  );
}
