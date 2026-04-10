<script setup lang="ts">
import { ref } from 'vue'

const filter = ref('')
const logs = ref<string[]>([
  '[INFO]  System ready.',
  '[INFO]  Waiting for device...',
])
</script>

<template>
  <div class="flex flex-col h-full space-y-4">
    <div class="flex items-center justify-between">
      <h1 class="text-xl font-bold text-gray-100">Log Viewer</h1>
      <button
        @click="logs = []"
        class="px-3 py-1.5 bg-gray-800 hover:bg-gray-700 rounded text-xs text-gray-400 transition-colors"
      >
        Clear
      </button>
    </div>

    <!-- Filter input -->
    <input
      v-model="filter"
      type="text"
      placeholder="Filter logs (keyword or /regex/)..."
      class="bg-gray-900 border border-gray-800 rounded px-3 py-2 text-sm text-gray-300 placeholder-gray-600 focus:outline-none focus:border-cyan-600 w-full"
    />

    <!-- Log terminal -->
    <div class="flex-1 bg-gray-900 border border-gray-800 rounded-lg p-4 overflow-auto font-mono text-xs leading-5">
      <div
        v-for="(line, i) in logs.filter(l => !filter || l.includes(filter))"
        :key="i"
        :class="[
          line.includes('[ERROR]') ? 'text-red-400' :
          line.includes('[WARN]')  ? 'text-yellow-400' :
          line.includes('[INFO]')  ? 'text-cyan-300' :
          'text-gray-400'
        ]"
      >
        {{ line }}
      </div>
      <div v-if="logs.length === 0" class="text-gray-600 italic">No log output yet.</div>
    </div>
  </div>
</template>
