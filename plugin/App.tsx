import React, {useSyncExternalStore} from 'react';
import {getScreen, subscribe} from './src/session';
import {AskPanel} from './src/ui/AskPanel';
import {DiagnosticsScreen} from './src/ui/DiagnosticsScreen';
import {SettingsScreen} from './src/ui/SettingsScreen';

function App(): React.JSX.Element {
  const {screen, seq} = useSyncExternalStore(subscribe, getScreen);
  if (screen === 'ask') {
    return <AskPanel />;
  }
  if (screen === 'settings') {
    return <SettingsScreen key={seq} />;
  }
  return <DiagnosticsScreen key={seq} />;
}

export default App;
