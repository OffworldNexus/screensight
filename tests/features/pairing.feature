Feature: Pairing a Screensight display
  As a Home Assistant user
  I want to pair a Screensight display discovered over mDNS
  So that I can show text on the panel

  Scenario: Pairing succeeds after the on-panel confirmation
    Given a Screensight device is advertising itself over mDNS in pairing mode
    When I submit the pairing code "93704101"
    And the device confirms the pairing on its panel
    Then a Screensight config entry exists with mutual static keys
