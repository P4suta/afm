// Read-only monospace view for a code/JSON string (the IR JSON tab).
// Mirrors the sibling aozora playground's CodeView.

import type { Component } from 'solid-js';

interface CodeViewProps {
  value: string;
}

const CodeView: Component<CodeViewProps> = (props) => {
  return <pre class="aozora-md-pg-code-view">{props.value}</pre>;
};

export default CodeView;
