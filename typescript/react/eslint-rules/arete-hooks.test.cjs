const { RuleTester } = require('eslint');
const rule = require('./arete-hooks.js');

const tester = new RuleTester({
  parser: require.resolve('@typescript-eslint/parser'),
  parserOptions: {
    ecmaVersion: 2022,
    sourceType: 'module',
    ecmaFeatures: { jsx: true },
  },
});

tester.run('arete-hooks', rule, {
  valid: [
    {
      code: `
        function Dashboard() {
          const arete = useArete(STACK);
          const round = arete.views.Round.state.use({ roundId: 1n });
          const mutation = arete.programs.ore.transactions.deploy.useMutation();
          return round.data ?? mutation.data;
        }
      `,
    },
    {
      code: `
        function useRound(arete) {
          return arete.views.Round.list.useOne();
        }
      `,
    },
    {
      code: `
        function Dashboard() {
          function format() { return 'value'; }
          const arete = useArete(STACK);
          const round = arete.views.Round.list.use();
          return format() + round.data;
        }
      `,
    },
    {
      code: `
        function Dashboard() {
          const model = useModel();
          const value = model.views.grid.use();
          return value;
        }
      `,
    },
  ],
  invalid: [
    {
      code: `
        function Dashboard({ enabled }) {
          const arete = useArete(STACK);
          if (enabled) {
            return arete.views.Round.list.use();
          }
          return null;
        }
      `,
      errors: [{ messageId: 'conditional' }],
    },
    {
      code: `
        function Dashboard({ ready }) {
          const arete = useArete(STACK);
          if (!ready) return null;
          const round = arete.views.Round.list.use();
          return round.data;
        }
      `,
      errors: [{ messageId: 'afterReturn' }],
    },
    {
      code: `
        function Dashboard() {
          const arete = useArete(STACK);
          const submit = () => arete.programs.ore.transactions.deploy.useMutation();
          return submit;
        }
      `,
      errors: [{ messageId: 'outsideComponent' }],
    },
    {
      code: `
        function Dashboard({ enabled }) {
          const arete = useArete(STACK);
          if (enabled) return arete.views.Round.state['use']({ roundId: 1n });
          return null;
        }
      `,
      errors: [{ messageId: 'conditional' }],
    },
  ],
});
