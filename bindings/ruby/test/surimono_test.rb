# frozen_string_literal: true

require "minitest/autorun"
require "surimono"

class SurimonoTest < Minitest::Test
  def test_native_version_matches_gem_version
    assert_equal Surimono::VERSION, Surimono.native_version
  end
end
