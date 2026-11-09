// Serves the site in dist/; files that exist are answered before this runs.
export default {
  fetch(request, env) {
    return env.ASSETS.fetch(request);
  },
};
