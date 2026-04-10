<script setup lang="ts">
</script>

<template>
  <div class="space-y-6 max-w-lg">
    <h1 class="text-xl font-bold text-gray-100">Settings</h1>

    <!-- Flash settings -->
    <section class="bg-gray-900 border border-gray-800 rounded-lg p-5 space-y-4">
      <h2 class="text-sm font-semibold text-cyan-400 uppercase tracking-wider">Flash</h2>

      <label class="flex items-center justify-between">
        <span class="text-sm text-gray-300">Auto-detect device on connect</span>
        <input type="checkbox" checked class="accent-cyan-500 w-4 h-4" />
      </label>

      <label class="flex items-center justify-between">
        <span class="text-sm text-gray-300">Verify after write</span>
        <input type="checkbox" checked class="accent-cyan-500 w-4 h-4" />
      </label>

      <label class="flex flex-col gap-1.5">
        <span class="text-sm text-gray-300">Default baud rate</span>
        <select class="bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 focus:outline-none w-48">
          <option>Auto</option>
          <option>115200</option>
          <option>921600</option>
          <option>3000000</option>
        </select>
      </label>
    </section>

    <!-- Log settings -->
    <section class="bg-gray-900 border border-gray-800 rounded-lg p-5 space-y-4">
      <h2 class="text-sm font-semibold text-cyan-400 uppercase tracking-wider">Log</h2>

      <label class="flex items-center justify-between">
        <span class="text-sm text-gray-300">Show timestamps</span>
        <input type="checkbox" class="accent-cyan-500 w-4 h-4" />
      </label>

      <label class="flex flex-col gap-1.5">
        <span class="text-sm text-gray-300">Max log lines</span>
        <input
          type="number"
          value="5000"
          class="bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 focus:outline-none w-32"
        />
      </label>
    </section>

    <!-- About -->
    <section class="bg-gray-900 border border-gray-800 rounded-lg p-5 space-y-1">
      <h2 class="text-sm font-semibold text-cyan-400 uppercase tracking-wider mb-2">About</h2>
      <p class="text-sm text-gray-400">LuaTools <span class="text-gray-200">v4</span></p>
      <p class="text-xs text-gray-600">Modern LuatOS flashing &amp; log tool · Vite + Vue 3 + Tailwind CSS</p>
    </section>
  </div>
</template>
