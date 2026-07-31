import '@react-spectrum/s2/page.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './styles/workspace.css';

const root = document.getElementById('app');
if (root === null) {
  throw new Error('#app missing from index.html');
}
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
