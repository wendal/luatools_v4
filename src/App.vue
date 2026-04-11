<script setup lang="ts">
import { ref, provide } from 'vue'
import FlashView from './views/FlashView.vue'
import LogView from './views/LogView.vue'
import FactoryView from './views/FactoryView.vue'
import SettingsView from './views/SettingsView.vue'

type ViewId = 'flash' | 'log' | 'factory' | 'settings'

const activeView = ref<ViewId>('flash')

const navItems: { id: ViewId; label: string; icon: string }[] = [
  { id: 'flash',    label: '下载 / 刷机', icon: '⚡' },
  { id: 'log',      label: '日志查看器',   icon: '📋' },
  { id: 'factory',  label: '量产模式',     icon: '🏭' },
  { id: 'settings', label: '设置',         icon: '⚙️' },
]

// ─── Shared cross-view state ──────────────────────────────────────────────────
const selectedPort = ref('')
const logBaud = ref(2000000)
// Set by FlashView to trigger auto-start of log capture in LogView
const pendingLogStart = ref<{ port: string; baud: number } | null>(null)

function switchToLog() {
  activeView.value = 'log'
}

function startLogCapture(port: string, baud: number) {
  pendingLogStart.value = { port, baud }
}

provide('sharedState', {
  selectedPort,
  logBaud,
  pendingLogStart,
  switchToLog,
  startLogCapture,
})


</script>

<template>
  <div class="flex flex-col h-screen bg-gray-950 text-gray-100 font-mono select-none overflow-hidden">

    <!-- ── Main layout ─────────────────────────────────────────────────── -->
    <div class="flex flex-1 overflow-hidden">

      <!-- Sidebar -->
      <aside class="w-44 shrink-0 flex flex-col bg-gray-900 border-r border-gray-800">
        <div class="px-4 py-4 border-b border-gray-800">
          <span class="text-lg font-bold tracking-wide text-cyan-400">LuaTools</span>
          <span class="ml-1 text-xs text-gray-500">v4</span>
        </div>

        <nav class="flex-1 py-2 space-y-0.5 px-2">
          <button
            v-for="item in navItems"
            :key="item.id"
            @click="activeView = item.id"
            :class="[
              'w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-sm transition-colors duration-100',
              activeView === item.id
                ? 'bg-cyan-600 text-white font-semibold'
                : 'text-gray-400 hover:bg-gray-800 hover:text-gray-100',
            ]"
          >
            <span class="text-base leading-none">{{ item.icon }}</span>
            <span>{{ item.label }}</span>
          </button>
        </nav>

        <!-- Port indicator in sidebar -->
        <div v-if="selectedPort" class="px-3 py-2 border-t border-gray-800 text-xs text-gray-500 truncate">
          📡 {{ selectedPort }}
        </div>
      </aside>

      <!-- Main content -->
      <main class="flex-1 overflow-auto bg-gray-950 p-5">
        <FlashView   v-if="activeView === 'flash'" />
        <LogView     v-else-if="activeView === 'log'" />
        <FactoryView v-else-if="activeView === 'factory'" />
        <SettingsView v-else-if="activeView === 'settings'" />
      </main>
    </div>

    <!-- ── Status bar ─────────────────────────────────────────────────── -->
    <footer class="shrink-0 flex items-center gap-4 px-4 py-1.5 bg-gray-900 border-t border-gray-800 text-xs text-gray-400">
      <span
        :class="selectedPort ? 'text-green-400' : 'text-gray-600'"
        class="flex items-center gap-1.5"
      >
        <span :class="selectedPort ? 'bg-green-400' : 'bg-gray-600'"
          class="inline-block w-2 h-2 rounded-full" />
        {{ selectedPort || '未连接' }}
      </span>
      <span v-if="logBaud !== 2000000" class="text-gray-600">{{ logBaud }} baud</span>
      <span class="ml-auto text-gray-700">LuatOS Flash Tool · BK7258 / Air8101</span>
    </footer>

  </div>
</template>

