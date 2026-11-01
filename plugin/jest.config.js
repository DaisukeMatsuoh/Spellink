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
  ],
};
