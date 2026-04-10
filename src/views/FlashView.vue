<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

interface PortInfo {
  port_name: string
  vid: string | null
  pid: string | null
  manufacturer: string | null
  product: string | null
  serial_number: string | null
}

const ports = ref<PortInfo[]>([])
const selectedPort = ref<string>('')
const portError = ref<string | null>(null)
const isLoadingPorts = ref(false)

async function refreshPorts() {
  isLoadingPorts.value = true
  portError.value = null
  try {
    const result = await invoke<PortInfo[]>('get_serial_ports')
    ports.value = result
    console.debug('[FlashView] get_serial_ports returned:', result)
    if (result.length === 0) {
      portError.value = 'No serial ports found. Connect a device and click Refresh.'
    }
    // Keep selection if the previously selected port is still in the list
    if (selectedPort.value && !result.find(p => p.port_name === selectedPort.value)) {
      selectedPort.value = ''
    }
  } catch (err) {
    portError.value = `Failed to enumerate serial ports: ${err}`
    console.error('[FlashView] get_serial_ports error:', err)
    ports.value = []
  } finally {
    isLoadingPorts.value = false
  }
}

onMounted(() => {
  refreshPorts()
})

function portLabel(p: PortInfo): string {
  const parts = [p.port_name]
  if (p.product) parts.push(p.product)
  else if (p.manufacturer) parts.push(p.manufacturer)
  if (p.vid && p.pid) parts.push(`[${p.vid}:${p.pid}]`)
  return parts.join(' — ')
}
</script>

<template>
  <div class="space-y-6">
    <h1 class="text-xl font-bold text-gray-100">Download / Flash</h1>

    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <!-- Firmware selection -->
      <div class="bg-gray-900 border border-gray-800 rounded-lg p-5 space-y-4">
        <h2 class="text-sm font-semibold text-cyan-400 uppercase tracking-wider">Core Firmware</h2>
        <div class="flex items-center gap-3">
          <input
            type="text"
            readonly
            placeholder="No file selected..."
            class="flex-1 bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 placeholder-gray-600 focus:outline-none"
          />
          <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm text-gray-200 transition-colors">
            Browse
          </button>
        </div>
      </div>

      <!-- Script / FS selection -->
      <div class="bg-gray-900 border border-gray-800 rounded-lg p-5 space-y-4">
        <h2 class="text-sm font-semibold text-cyan-400 uppercase tracking-wider">Script / File System</h2>
        <div class="flex items-center gap-3">
          <input
            type="text"
            readonly
            placeholder="Drag a folder or select files..."
            class="flex-1 bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 placeholder-gray-600 focus:outline-none"
          />
          <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm text-gray-200 transition-colors">
            Browse
          </button>
        </div>
      </div>
    </div>

    <!-- Device / port -->
    <div class="bg-gray-900 border border-gray-800 rounded-lg p-5 space-y-3">
      <h2 class="text-sm font-semibold text-cyan-400 uppercase tracking-wider">Device</h2>
      <div class="flex items-center gap-3">
        <select
          v-model="selectedPort"
          class="bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-300 focus:outline-none min-w-[220px]"
          :disabled="isLoadingPorts"
        >
          <option value="">— Auto detect —</option>
          <option v-for="p in ports" :key="p.port_name" :value="p.port_name">
            {{ portLabel(p) }}
          </option>
        </select>
        <button
          class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm text-gray-200 transition-colors disabled:opacity-50"
          :disabled="isLoadingPorts"
          @click="refreshPorts"
        >
          {{ isLoadingPorts ? '...' : 'Refresh' }}
        </button>
      </div>
      <p v-if="portError" class="text-xs text-yellow-400">{{ portError }}</p>
    </div>

    <!-- Flash button -->
    <button
      class="w-full py-3 bg-cyan-600 hover:bg-cyan-500 active:bg-cyan-700 rounded-lg text-base font-bold text-white transition-colors"
    >
      ⚡ Start Flash
    </button>

    <!-- Progress placeholder -->
    <div class="bg-gray-900 border border-gray-800 rounded-lg p-5">
      <h2 class="text-sm font-semibold text-cyan-400 uppercase tracking-wider mb-3">Progress</h2>
      <div class="w-full bg-gray-800 rounded-full h-2">
        <div class="bg-cyan-500 h-2 rounded-full w-0 transition-all duration-300" />
      </div>
      <p class="mt-2 text-xs text-gray-500">Waiting to start...</p>
    </div>
  </div>
</template>
