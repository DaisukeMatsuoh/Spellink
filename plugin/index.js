import {AppRegistry, Image} from 'react-native';
import {PluginManager} from 'sn-plugin-lib';
import {name as appName} from './app.json';
import {buttonName, startLocaleTracking} from './src/i18n';
import {show} from './src/session';

const SIDEBAR = 1;
const LASSO_TOOLBAR = 2;
const EDIT_DATA_STROKE = 0;
const SHOW_UI = 1;
const NO_UI = 0;

const ASK_BUTTON_ID = 200;
const DIAGNOSTICS_BUTTON_ID = 900;

// The UI (and the crypto it pulls in) loads only when a button opens it, so a failure there
// can never stop the buttons below from being registered.
AppRegistry.registerComponent(appName, () => require('./App').default);

PluginManager.init();
startLocaleTracking();

const icon = Image.resolveAssetSource(require('./assets/icon.png')).uri;

PluginManager.registerButton(LASSO_TOOLBAR, ['NOTE'], {
  id: ASK_BUTTON_ID,
  name: buttonName('askButton'),
  icon,
  editDataTypes: [EDIT_DATA_STROKE],
  // No UI: the answer is written while the note stays in view. The progress panel is opened
  // by the controller only when the user enabled it or something went wrong.
  showType: NO_UI,
});

PluginManager.registerButton(SIDEBAR, ['NOTE'], {
  id: DIAGNOSTICS_BUTTON_ID,
  name: buttonName('diagButton'),
  icon,
  showType: SHOW_UI,
});

PluginManager.registerButtonListener({
  onButtonPress(event) {
    if (event.id === ASK_BUTTON_ID) {
      require('./src/askController').startAsk();
    } else if (event.id === DIAGNOSTICS_BUTTON_ID) {
      show('diagnostics');
    }
  },
});

// "Settings" entry on the plugin's card in Settings → Apps → Plugins.
PluginManager.registerConfigButton();
PluginManager.registerConfigButtonListener({
  onClick() {
    show('settings');
    PluginManager.showPluginView();
  },
});
