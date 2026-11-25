import { createRoot } from 'react-dom/client';
import { RuntimeStatusClient } from '../../adapters/runtimeStatus';
import { StatusPage } from '../../ui/pages/StatusPage';

createRoot(document.getElementById('root')!).render(<StatusPage client={new RuntimeStatusClient()} />);
