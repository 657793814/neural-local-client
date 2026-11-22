import {createRoot} from 'react-dom/client';
import 'katex/dist/katex.min.css';
import './styles/app.css';
import './styles/paper.css';
import App from './App.jsx';
import ErrorBoundary from './components/ErrorBoundary.jsx';

createRoot(document.getElementById('root')).render(
    <ErrorBoundary>
        <App/>
    </ErrorBoundary>,
);
