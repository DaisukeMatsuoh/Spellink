module.exports = {
  preset: 'react-native',
  // The crypto libraries ship as ES modules only.
  transformIgnorePatterns: [
    'node_modules/(?!((jest-)?react-native|@react-native(-community)?|@noble)/)',
  ],
  testMatch: ['**/__tests__/**/*.test.ts'],
  collectCoverageFrom: [
    'src/freeSpace.ts',
    'src/inkDrops.ts',
    'src/noise.ts',
    'src/utf8.ts',
    'src/askFlow.ts',
    'src/i18n.ts',
    'src/base64url.ts',
    'src/pairConfig.ts',
    'src/pairStore.ts',
    'src/pairFrames.ts',
    'src/pairPlan.ts',
    'src/pairing.ts',
    'src/pairingMachine.ts',
    'src/pairConnect.ts',
    'src/pairText.ts',
    'src/pairStroke.ts',
    'src/fileOps.ts',
    'src/pairingState.ts',
    'src/bridgeConfig.ts',
  ],
};
