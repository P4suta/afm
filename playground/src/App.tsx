import { PlaygroundApp } from '@aozora/playground-ui';

import { afmPlaygroundAdapter } from './adapter';

export default function App() {
  return <PlaygroundApp adapter={afmPlaygroundAdapter} />;
}
