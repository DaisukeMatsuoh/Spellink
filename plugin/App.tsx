import React, {useSyncExternalStore} from 'react';
import {getScreen, subscribe} from './src/session';
import {AskPanel} from './src/ui/AskPanel';
import {DiagnosticsScreen} from './src/ui/DiagnosticsScreen';

function App(): React.JSX.Element {
  const {screen, seq} = useSyncExternalStore(subscribe, getScreen);
  return screen === 'ask' ? (
    <AskPanel seq={seq} />
  ) : (
    <DiagnosticsScreen key={seq} />
  );
}

export default App;
