import React from 'react';
import ReactDOM from 'react-dom/client';
import DesktopApp from './desktop/DesktopApp.tsx';

const upstream = new URLSearchParams(location.search).has('upstream');
const App = upstream
  ? React.lazy(async () => (await Promise.all([import('./App.tsx'), import('./index.css'), import('./i18n.ts')]))[0])
  : DesktopApp;

// eslint-disable-next-line @typescript-eslint/no-non-null-assertion
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <React.Suspense fallback={null}><App /></React.Suspense>
  </React.StrictMode>,
);
