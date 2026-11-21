import React from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import './styles/globals.css';
import { suppressBrowserMenu } from './lib/browser-menu';
import { syncDisplayPathClass } from './lib/display-path';

suppressBrowserMenu(document);
syncDisplayPathClass();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {/* Session routes and the external session registry must commit together
        when closing a tab; deferred navigation exposes a stale session route. */}
    <BrowserRouter useTransitions={false}>
      <App />
    </BrowserRouter>
  </React.StrictMode>
);
