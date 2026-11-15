import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource/golos-text/400.css';
import '@fontsource/golos-text/500.css';
import '@fontsource/golos-text/600.css';
import '@fontsource/geologica/300.css';
import '@fontsource/geologica/400.css';
import '@fontsource/geologica/500.css';
import './styles.css';
import App from './App';
import TitleBar, { customTitleBar } from './TitleBar';

// Своя полоса окна — сразу, чтобы раскладка не прыгала.
if (customTitleBar) document.documentElement.classList.add('own-titlebar');

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TitleBar />
    <App />
  </StrictMode>,
);
