import { beforeEach, describe, expect, it } from 'vitest'
import { useEditor } from './editor'

const f = (id: string) => ({ kind: 'furniture' as const, id })

describe('multi selection', () => {
  beforeEach(() => useEditor.getState().select(null))

  it('select replaces the set', () => {
    const ed = useEditor.getState()
    ed.select(f('a'))
    ed.select(f('b'))
    expect(useEditor.getState().multi).toEqual([f('b')])
    expect(useEditor.getState().selection).toEqual(f('b'))
  })

  it('toggle adds and removes, primary is the last one added', () => {
    const ed = useEditor.getState()
    ed.select(f('a'))
    ed.toggleSelect(f('b'))
    expect(useEditor.getState().multi).toEqual([f('a'), f('b')])
    expect(useEditor.getState().selection).toEqual(f('b'))
    ed.toggleSelect(f('b'))
    expect(useEditor.getState().multi).toEqual([f('a')])
    expect(useEditor.getState().selection).toEqual(f('a'))
    ed.toggleSelect(f('a'))
    expect(useEditor.getState().selection).toBeNull()
  })

  it('selectMany sets all, primary last', () => {
    useEditor.getState().selectMany([f('a'), f('b')])
    expect(useEditor.getState().multi).toHaveLength(2)
    expect(useEditor.getState().selection).toEqual(f('b'))
    useEditor.getState().selectMany([])
    expect(useEditor.getState().selection).toBeNull()
  })
})
